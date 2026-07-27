//! DNS resource record data (RDATA) implementation
//!
//! This module defines the RDATA types for various DNS record types.
//! RDATA contains the actual data for a DNS resource record.

use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};

/// DNS resource record data
///
/// Contains the actual data for different types of DNS records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RData {
    /// IPv4 address (A record)
    A(Ipv4Addr),

    /// IPv6 address (AAAA record)
    AAAA(Ipv6Addr),

    /// Canonical name (CNAME record)
    CNAME(String),

    /// Mail exchange (MX record)
    MX {
        /// Preference value for this MX record
        preference: u16,
        /// Mail exchange hostname
        exchange: String,
    },

    /// Name server (NS record)
    NS(String),

    /// Pointer (PTR record)
    PTR(String),

    /// Text (TXT record)
    TXT(Vec<String>),

    /// Start of authority (SOA record)
    SOA {
        /// Primary name server
        mname: String,
        /// Responsible person's email
        rname: String,
        /// Serial number
        serial: u32,
        /// Refresh interval
        refresh: u32,
        /// Retry interval
        retry: u32,
        /// Expiration time
        expire: u32,
        /// Minimum TTL
        minimum: u32,
    },

    /// Service record (SRV record)
    SRV {
        /// Priority of this target
        priority: u16,
        /// Weight for records with same priority
        weight: u16,
        /// Port number of the service
        port: u16,
        /// Target hostname
        target: String,
    },

    /// Certificate authority authorization (CAA record)
    CAA {
        /// Flags byte
        flags: u8,
        /// Property tag
        tag: String,
        /// Property value
        value: String,
    },

    /// Service binding (SVCB record) - RFC 9460
    SVCB {
        /// Priority (0 for alias mode, non-zero for service mode)
        priority: u16,
        /// Target domain name
        target: String,
        /// Service parameters as raw bytes (simplified)
        params: Vec<u8>,
    },

    /// HTTPS service binding (HTTPS record) - RFC 9460
    /// Semantically identical to SVCB but for HTTPS
    HTTPS {
        /// Priority (0 for alias mode, non-zero for service mode)
        priority: u16,
        /// Target domain name
        target: String,
        /// Service parameters as raw bytes (simplified)
        params: Vec<u8>,
    },

    /// DNSSEC delegation signer (DS record) - RFC 4034
    DS {
        /// Key tag
        key_tag: u16,
        /// Algorithm
        algorithm: u8,
        /// Digest type
        digest_type: u8,
        /// Digest
        digest: Vec<u8>,
    },

    /// DNSSEC signature (RRSIG record) - RFC 4034
    RRSIG {
        /// Type covered
        type_covered: u16,
        /// Algorithm
        algorithm: u8,
        /// Labels
        labels: u8,
        /// Original TTL
        original_ttl: u32,
        /// Signature expiration
        expiration: u32,
        /// Signature inception
        inception: u32,
        /// Key tag
        key_tag: u16,
        /// Signer's name
        signer_name: String,
        /// Signature
        signature: Vec<u8>,
    },

    /// Next secure record (NSEC) - RFC 4034
    NSEC {
        /// Next domain name
        next_domain: String,
        /// Type bit maps
        type_bitmaps: Vec<u8>,
    },

    /// DNSSEC key (DNSKEY record) - RFC 4034
    DNSKEY {
        /// Flags
        flags: u16,
        /// Protocol (must be 3)
        protocol: u8,
        /// Algorithm
        algorithm: u8,
        /// Public key
        public_key: Vec<u8>,
    },

    /// Next secure record v3 (NSEC3) - RFC 5155
    NSEC3 {
        /// Hash algorithm
        hash_algorithm: u8,
        /// Flags
        flags: u8,
        /// Iterations
        iterations: u16,
        /// Salt
        salt: Vec<u8>,
        /// Next hashed owner name
        next_hashed: Vec<u8>,
        /// Type bit maps
        type_bitmaps: Vec<u8>,
    },

    /// NSEC3 parameters (NSEC3PARAM) - RFC 5155
    NSEC3PARAM {
        /// Hash algorithm
        hash_algorithm: u8,
        /// Flags
        flags: u8,
        /// Iterations
        iterations: u16,
        /// Salt
        salt: Vec<u8>,
    },

    /// OPT pseudo-record for EDNS(0) - RFC 6891
    OPT {
        /// Extended RCODE
        extended_rcode: u8,
        /// EDNS version
        version: u8,
        /// EDNS flags (DO bit, and others)
        flags: u16,
        /// EDNS options as raw bytes
        options: Vec<u8>,
    },

    /// Unknown or raw record data
    Unknown(Vec<u8>),
}

impl RData {
    /// Create an A record with an IPv4 address
    pub fn a(addr: Ipv4Addr) -> Self {
        RData::A(addr)
    }

    /// Create an AAAA record with an IPv6 address
    pub fn aaaa(addr: Ipv6Addr) -> Self {
        RData::AAAA(addr)
    }

    /// Create a CNAME record
    pub fn cname(name: String) -> Self {
        RData::CNAME(name)
    }

    /// Create an MX record
    pub fn mx(preference: u16, exchange: String) -> Self {
        RData::MX {
            preference,
            exchange,
        }
    }

    /// Create an NS record
    pub fn ns(name: String) -> Self {
        RData::NS(name)
    }

    /// Create a PTR record
    pub fn ptr(name: String) -> Self {
        RData::PTR(name)
    }

    /// Create a TXT record
    pub fn txt(texts: Vec<String>) -> Self {
        RData::TXT(texts)
    }

    /// Create an SOA record
    pub fn soa(
        mname: String,
        rname: String,
        serial: u32,
        refresh: u32,
        retry: u32,
        expire: u32,
        minimum: u32,
    ) -> Self {
        RData::SOA {
            mname,
            rname,
            serial,
            refresh,
            retry,
            expire,
            minimum,
        }
    }

    /// Create an SRV record
    pub fn srv(priority: u16, weight: u16, port: u16, target: String) -> Self {
        RData::SRV {
            priority,
            weight,
            port,
            target,
        }
    }

    /// Create a CAA record
    pub fn caa(flags: u8, tag: String, value: String) -> Self {
        RData::CAA { flags, tag, value }
    }

    /// Create an SVCB record
    pub fn svcb(priority: u16, target: String, params: Vec<u8>) -> Self {
        RData::SVCB {
            priority,
            target,
            params,
        }
    }

    /// Create an HTTPS record
    pub fn https(priority: u16, target: String, params: Vec<u8>) -> Self {
        RData::HTTPS {
            priority,
            target,
            params,
        }
    }
}

impl fmt::Display for RData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RData::A(addr) => write!(f, "{}", addr),
            RData::AAAA(addr) => write!(f, "{}", addr),
            RData::CNAME(name) => write!(f, "{}", name),
            RData::MX {
                preference,
                exchange,
            } => write!(f, "{} {}", preference, exchange),
            RData::NS(name) => write!(f, "{}", name),
            RData::PTR(name) => write!(f, "{}", name),
            RData::TXT(texts) => {
                let joined = texts
                    .iter()
                    .map(|s| format!("\"{}\"", s))
                    .collect::<Vec<_>>()
                    .join(" ");
                write!(f, "{}", joined)
            }
            RData::SOA {
                mname,
                rname,
                serial,
                refresh,
                retry,
                expire,
                minimum,
            } => write!(
                f,
                "{} {} {} {} {} {} {}",
                mname, rname, serial, refresh, retry, expire, minimum
            ),
            RData::SRV {
                priority,
                weight,
                port,
                target,
            } => write!(f, "{} {} {} {}", priority, weight, port, target),
            RData::CAA { flags, tag, value } => write!(f, "{} {} \"{}\"", flags, tag, value),
            RData::SVCB {
                priority,
                target,
                params,
            } => write!(
                f,
                "{} {} <params: {} bytes>",
                priority,
                target,
                params.len()
            ),
            RData::HTTPS {
                priority,
                target,
                params,
            } => write!(
                f,
                "{} {} <params: {} bytes>",
                priority,
                target,
                params.len()
            ),
            RData::DS {
                key_tag,
                algorithm,
                digest_type,
                digest,
            } => write!(
                f,
                "{} {} {} <digest: {} bytes>",
                key_tag,
                algorithm,
                digest_type,
                digest.len()
            ),
            RData::RRSIG {
                type_covered,
                algorithm,
                signer_name,
                ..
            } => write!(f, "{} {} {} ...", type_covered, algorithm, signer_name),
            RData::NSEC {
                next_domain,
                type_bitmaps,
            } => write!(f, "{} <{} types>", next_domain, type_bitmaps.len()),
            RData::DNSKEY {
                flags,
                protocol,
                algorithm,
                public_key,
            } => write!(
                f,
                "{} {} {} <key: {} bytes>",
                flags,
                protocol,
                algorithm,
                public_key.len()
            ),
            RData::NSEC3 {
                hash_algorithm,
                iterations,
                ..
            } => write!(f, "{} {} ...", hash_algorithm, iterations),
            RData::NSEC3PARAM {
                hash_algorithm,
                iterations,
                ..
            } => write!(f, "{} {} ...", hash_algorithm, iterations),
            RData::OPT {
                version,
                flags,
                options,
                ..
            } => write!(
                f,
                "EDNS v{} flags:{:#x} <{} bytes>",
                version,
                flags,
                options.len()
            ),
            RData::Unknown(data) => write!(f, "<{} bytes>", data.len()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_a_record() {
        let ip = Ipv4Addr::from_str("192.0.2.1").unwrap();
        let rdata = RData::a(ip);

        assert_eq!(rdata, RData::A(ip));
        assert_eq!(format!("{}", rdata), "192.0.2.1");
    }

    #[test]
    fn test_aaaa_record() {
        let ip = Ipv6Addr::from_str("2001:db8::1").unwrap();
        let rdata = RData::aaaa(ip);

        assert_eq!(rdata, RData::AAAA(ip));
        assert_eq!(format!("{}", rdata), "2001:db8::1");
    }

    #[test]
    fn test_cname_record() {
        let rdata = RData::cname("example.com".to_string());

        assert_eq!(rdata, RData::CNAME("example.com".to_string()));
        assert_eq!(format!("{}", rdata), "example.com");
    }

    #[test]
    fn test_mx_record() {
        let rdata = RData::mx(10, "mail.example.com".to_string());

        if let RData::MX {
            preference,
            exchange,
        } = &rdata
        {
            assert_eq!(*preference, 10);
            assert_eq!(exchange, "mail.example.com");
        } else {
            panic!("Expected MX record");
        }

        assert_eq!(format!("{}", rdata), "10 mail.example.com");
    }

    #[test]
    fn test_txt_record() {
        let rdata = RData::txt(vec![
            "v=spf1".to_string(),
            "include:example.com".to_string(),
        ]);

        let display = format!("{}", rdata);
        assert!(display.contains("v=spf1"));
        assert!(display.contains("include:example.com"));
    }

    #[test]
    fn test_srv_record() {
        let rdata = RData::srv(10, 60, 5060, "sipserver.example.com".to_string());

        if let RData::SRV {
            priority,
            weight,
            port,
            target,
        } = &rdata
        {
            assert_eq!(*priority, 10);
            assert_eq!(*weight, 60);
            assert_eq!(*port, 5060);
            assert_eq!(target, "sipserver.example.com");
        } else {
            panic!("Expected SRV record");
        }
    }

    #[test]
    fn test_ns_record() {
        let rdata = RData::ns("ns1.example.com".to_string());
        assert_eq!(format!("{}", rdata), "ns1.example.com");
    }

    #[test]
    fn test_ptr_record() {
        let rdata = RData::ptr("example.com".to_string());
        assert_eq!(format!("{}", rdata), "example.com");
    }

    #[test]
    fn test_svcb_record() {
        let rdata = RData::svcb(1, "example.com".to_string(), vec![1, 2, 3]);

        if let RData::SVCB {
            priority,
            target,
            params,
        } = &rdata
        {
            assert_eq!(*priority, 1);
            assert_eq!(target, "example.com");
            assert_eq!(params, &vec![1, 2, 3]);
        } else {
            panic!("Expected SVCB record");
        }

        assert!(format!("{}", rdata).contains("example.com"));
        assert!(format!("{}", rdata).contains("3 bytes"));
    }

    #[test]
    fn test_https_record() {
        let rdata = RData::https(1, "example.com".to_string(), vec![4, 5, 6]);

        if let RData::HTTPS {
            priority,
            target,
            params,
        } = &rdata
        {
            assert_eq!(*priority, 1);
            assert_eq!(target, "example.com");
            assert_eq!(params, &vec![4, 5, 6]);
        } else {
            panic!("Expected HTTPS record");
        }

        assert!(format!("{}", rdata).contains("example.com"));
        assert!(format!("{}", rdata).contains("3 bytes"));
    }

    #[test]
    fn test_soa_record() {
        let rdata = RData::soa(
            "ns1.example.com".to_string(),
            "admin.example.com".to_string(),
            2024010101,
            3600,
            600,
            604800,
            86400,
        );

        if let RData::SOA {
            mname,
            rname,
            serial,
            refresh,
            retry,
            expire,
            minimum,
        } = &rdata
        {
            assert_eq!(mname, "ns1.example.com");
            assert_eq!(rname, "admin.example.com");
            assert_eq!(*serial, 2024010101);
            assert_eq!(*refresh, 3600);
            assert_eq!(*retry, 600);
            assert_eq!(*expire, 604800);
            assert_eq!(*minimum, 86400);
        } else {
            panic!("Expected SOA record");
        }

        let display = format!("{}", rdata);
        assert!(display.contains("ns1.example.com"));
        assert!(display.contains("admin.example.com"));
    }

    #[test]
    fn test_caa_record() {
        let rdata = RData::caa(0, "issue".to_string(), "letsencrypt.org".to_string());

        if let RData::CAA { flags, tag, value } = &rdata {
            assert_eq!(*flags, 0);
            assert_eq!(tag, "issue");
            assert_eq!(value, "letsencrypt.org");
        } else {
            panic!("Expected CAA record");
        }

        let display = format!("{}", rdata);
        assert!(display.contains("issue"));
        assert!(display.contains("letsencrypt.org"));
    }

    #[test]
    fn test_unknown_record() {
        let rdata = RData::Unknown(vec![0x01, 0x02, 0x03]);
        let display = format!("{}", rdata);
        assert!(display.contains("3 bytes"));
    }

    #[test]
    fn test_ds_record() {
        let rdata = RData::DS {
            key_tag: 12345,
            algorithm: 8,
            digest_type: 2,
            digest: vec![0xab, 0xcd],
        };
        let display = format!("{}", rdata);
        assert!(display.contains("12345"));
    }

    #[test]
    fn test_dnskey_record() {
        let rdata = RData::DNSKEY {
            flags: 257,
            protocol: 3,
            algorithm: 8,
            public_key: vec![0x01, 0x02],
        };
        let display = format!("{}", rdata);
        assert!(display.contains("257"));
    }

    #[test]
    fn test_nsec_record() {
        let rdata = RData::NSEC {
            next_domain: "example.com".to_string(),
            type_bitmaps: vec![0x01],
        };
        let display = format!("{}", rdata);
        assert!(display.contains("example.com"));
    }

    #[test]
    fn test_nsec3_record() {
        let rdata = RData::NSEC3 {
            hash_algorithm: 1,
            flags: 0,
            iterations: 10,
            salt: vec![0xab],
            next_hashed: vec![0xcd],
            type_bitmaps: vec![0x01],
        };
        let display = format!("{}", rdata);
        // Display format is "hash_algorithm iterations ..."
        assert!(display.contains("1"));
        assert!(display.contains("10"));
    }

    #[test]
    fn test_nsec3param_record() {
        let rdata = RData::NSEC3PARAM {
            hash_algorithm: 1,
            flags: 0,
            iterations: 10,
            salt: vec![0xab],
        };
        let display = format!("{}", rdata);
        // Display format is "hash_algorithm iterations ..."
        assert!(display.contains("1"));
        assert!(display.contains("10"));
    }

    #[test]
    fn test_rrsig_record() {
        let rdata = RData::RRSIG {
            type_covered: 1,
            algorithm: 8,
            labels: 2,
            original_ttl: 3600,
            expiration: 1234567890,
            inception: 1234500000,
            key_tag: 12345,
            signer_name: "example.com".to_string(),
            signature: vec![0x01, 0x02],
        };
        let display = format!("{}", rdata);
        assert!(display.contains("example.com"));
    }

    #[test]
    fn test_opt_record() {
        let rdata = RData::OPT {
            extended_rcode: 0,
            version: 0,
            flags: 0x8000, // DO bit
            options: vec![],
        };
        let display = format!("{}", rdata);
        assert!(display.contains("EDNS"));
        assert!(display.contains("v0"));
    }

    #[test]
    fn test_rdata_clone() {
        let original = RData::A(Ipv4Addr::new(1, 2, 3, 4));
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    #[test]
    fn test_rdata_eq() {
        let rdata1 = RData::A(Ipv4Addr::new(1, 2, 3, 4));
        let rdata2 = RData::A(Ipv4Addr::new(1, 2, 3, 4));
        let rdata3 = RData::A(Ipv4Addr::new(5, 6, 7, 8));

        assert_eq!(rdata1, rdata2);
        assert_ne!(rdata1, rdata3);
    }

    #[test]
    fn test_rdata_debug() {
        let rdata = RData::A(Ipv4Addr::new(192, 168, 1, 1));
        let debug_str = format!("{:?}", rdata);
        assert!(debug_str.contains("A"));
        assert!(debug_str.contains("192"));
    }
}
