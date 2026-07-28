use std::io::{Read, Write};
use std::path::Path;

use super::CacheEntry;
use crate::Result;
use crate::dns::{Message, wire};

const MAGIC: &[u8; 8] = b"LZDNSCv1";
const DUMP_THRESHOLD: u64 = 1024;

/// Entry loaded from a dump file, before reconstruction into CacheEntry.
pub struct PersistedEntry {
    pub key: String,
    pub response: Message,
    pub cached_at_unix: u64,
    pub original_ttl: u32,
}

/// Dump cache entries to a binary file. Atomically writes via temp file + rename.
pub fn dump_cache(path: &Path, entries: &[(String, CacheEntry)]) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }

    let tmp = path.with_extension("tmp");
    let mut file = std::fs::File::create(&tmp)?;

    file.write_all(MAGIC)?;

    let count = entries.len() as u32;
    file.write_all(&count.to_le_bytes())?;

    for (key, entry) in entries {
        let key_bytes = key.as_bytes();
        file.write_all(&(key_bytes.len() as u16).to_le_bytes())?;
        file.write_all(key_bytes)?;

        file.write_all(&entry.cached_at_unix.to_le_bytes())?;
        file.write_all(&entry.original_ttl.to_le_bytes())?;

        let msg_bytes = wire::serialize_message(&entry.response)?;
        file.write_all(&(msg_bytes.len() as u32).to_le_bytes())?;
        file.write_all(&msg_bytes)?;
    }

    file.flush()?;
    drop(file);

    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Load cache entries from a dump file. Skips entries that have fully expired.
pub fn load_cache(path: &Path) -> Result<Vec<PersistedEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let mut file = std::fs::File::open(path)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;

    if buf.len() < 12 {
        return Ok(Vec::new());
    }

    let magic = &buf[..8];
    if magic != MAGIC {
        return Err(crate::Error::Config(format!(
            "invalid cache dump magic: expected {:?}, got {:?}",
            MAGIC, magic
        )));
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut pos = 8;
    let count = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
    pos += 4;

    let mut entries = Vec::with_capacity(count.min(8192));

    for _ in 0..count {
        if pos + 2 > buf.len() {
            break;
        }
        let key_len = u16::from_le_bytes(buf[pos..pos + 2].try_into().unwrap()) as usize;
        pos += 2;

        if pos + key_len > buf.len() {
            break;
        }
        let key = String::from_utf8_lossy(&buf[pos..pos + key_len]).to_string();
        pos += key_len;

        if pos + 12 > buf.len() {
            break;
        }
        let cached_at_unix = u64::from_le_bytes(buf[pos..pos + 8].try_into().unwrap());
        pos += 8;
        let original_ttl = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap());
        pos += 4;

        if pos + 4 > buf.len() {
            break;
        }
        let msg_len = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;

        if pos + msg_len > buf.len() {
            break;
        }
        let msg_bytes = &buf[pos..pos + msg_len];
        pos += msg_len;

        let elapsed = now.saturating_sub(cached_at_unix);
        if elapsed >= original_ttl as u64 {
            continue;
        }

        match wire::parse_message(msg_bytes) {
            Ok(response) => entries.push(PersistedEntry {
                key,
                response,
                cached_at_unix,
                original_ttl,
            }),
            Err(e) => {
                tracing::warn!(error = %e, "skipping unparseable cache entry");
            }
        }
    }

    Ok(entries)
}

/// Threshold for change-count triggered periodic dumps.
pub fn dump_threshold() -> u64 {
    DUMP_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::{Message, Question, RecordClass, RecordType};
    use std::time::Instant;

    fn make_entry(qname: &str, ttl: u32) -> (String, CacheEntry) {
        let mut msg = Message::new();
        msg.add_question(Question::new(qname, RecordType::A, RecordClass::IN));
        msg.set_response(true);

        let key = format!("{}:1:1:0", qname);
        let entry = CacheEntry {
            response: std::sync::Arc::new(msg),
            cached_at: Instant::now(),
            ttl,
            cache_ttl: ttl,
            original_ttl: ttl,
            last_accessed: Instant::now(),
            cached_at_unix: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        };
        (key, entry)
    }

    #[test]
    fn roundtrip() {
        let dir = std::env::temp_dir();
        let path = dir.join("lazydns_cache_test_rt.dump");

        let entries = vec![make_entry("example.com", 300), make_entry("test.org", 600)];

        dump_cache(&path, &entries).unwrap();
        let loaded = load_cache(&path).unwrap();

        assert_eq!(loaded.len(), 2);
        assert!(loaded.iter().any(|e| e.key == "example.com:1:1:0"));
        assert!(loaded.iter().any(|e| e.key == "test.org:1:1:0"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_missing_file() {
        let path = std::env::temp_dir().join("lazydns_nonexistent.dump");
        let loaded = load_cache(&path).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn dump_creates_parent_dir() {
        let dir = std::env::temp_dir().join("lazydns_cache_test_nested");
        let path = dir.join("sub").join("cache.dump");

        let entries = vec![make_entry("nested.example", 300)];
        dump_cache(&path, &entries).unwrap();
        assert!(path.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_skips_expired() {
        let dir = std::env::temp_dir();
        let path = dir.join("lazydns_cache_test_exp.dump");

        // Entry cached far in the past with TTL 1s: definitely expired by now.
        let mut msg = Message::new();
        msg.add_question(Question::new("expired.com", RecordType::A, RecordClass::IN));
        msg.set_response(true);

        let entry = CacheEntry {
            response: std::sync::Arc::new(msg),
            cached_at: Instant::now(),
            ttl: 1,
            cache_ttl: 1,
            original_ttl: 1,
            last_accessed: Instant::now(),
            cached_at_unix: 1, // year 1970
        };

        dump_cache(&path, &[("expired.com:1:1:0".to_string(), entry)]).unwrap();
        let loaded = load_cache(&path).unwrap();
        assert!(loaded.is_empty());

        let _ = std::fs::remove_file(&path);
    }
}
