use crate::RegisterExecPlugin;
use crate::dns::RecordType;
use crate::plugin::{ExecPlugin, Plugin};
use async_trait::async_trait;
use std::sync::Arc;

#[derive(Debug, Default, Clone, Copy, RegisterExecPlugin)]
pub struct PreferIpv4Plugin;

impl PreferIpv4Plugin {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Plugin for PreferIpv4Plugin {
    fn name(&self) -> &str {
        "prefer_ipv4"
    }

    async fn execute(&self, ctx: &mut crate::plugin::Context) -> crate::Result<()> {
        // Only filter AAAA records if the query type is A
        // If query type is AAAA, we should NOT filter AAAA records
        let should_filter = ctx
            .request()
            .questions()
            .first()
            .map(|q| q.qtype() == RecordType::A)
            .unwrap_or(false);

        if should_filter && let Some(response) = ctx.response_mut() {
            let answers = response.answers_mut();
            answers.retain(|record| !matches!(record.rtype(), RecordType::AAAA));

            let additional = response.additional_mut();
            additional.retain(|record| !matches!(record.rtype(), RecordType::AAAA));
        }

        Ok(())
    }
}

impl ExecPlugin for PreferIpv4Plugin {
    fn quick_setup(prefix: &str, _exec_str: &str) -> crate::Result<Arc<dyn Plugin>> {
        if prefix != "prefer_ipv4" {
            return Err(crate::Error::Config(format!(
                "ExecPlugin quick_setup: unsupported prefix '{}', expected 'prefer_ipv4'",
                prefix
            )));
        }

        Ok(Arc::new(PreferIpv4Plugin::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::{Message, Question, RData, RecordClass, ResourceRecord};

    #[tokio::test]
    async fn test_prefer_ipv4_plugin_filters_on_a_query() {
        let plugin = PreferIpv4Plugin::new();
        assert_eq!(plugin.name(), "prefer_ipv4");

        // Test with A query - should filter AAAA records
        let mut request = Message::new();
        request.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));

        let mut response = Message::new();
        response.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));

        response.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::A,
            RecordClass::IN,
            300,
            RData::A("192.0.2.1".parse().unwrap()),
        ));

        response.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::AAAA,
            RecordClass::IN,
            300,
            RData::AAAA("2001:db8::1".parse().unwrap()),
        ));

        let mut ctx = crate::plugin::Context::new(request);
        ctx.set_response(Some(response));
        plugin.execute(&mut ctx).await.unwrap();

        let response = ctx.response().unwrap();
        assert_eq!(response.answers().len(), 1);
        assert!(matches!(response.answers()[0].rtype(), RecordType::A));
    }

    #[tokio::test]
    async fn test_prefer_ipv4_plugin_preserves_on_aaaa_query() {
        let plugin = PreferIpv4Plugin::new();

        // Test with AAAA query - should NOT filter AAAA records
        let mut request = Message::new();
        request.add_question(Question::new(
            "example.com",
            RecordType::AAAA,
            RecordClass::IN,
        ));

        let mut response = Message::new();
        response.add_question(Question::new(
            "example.com",
            RecordType::AAAA,
            RecordClass::IN,
        ));

        response.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::A,
            RecordClass::IN,
            300,
            RData::A("192.0.2.1".parse().unwrap()),
        ));

        response.add_answer(ResourceRecord::new(
            "example.com",
            RecordType::AAAA,
            RecordClass::IN,
            300,
            RData::AAAA("2001:db8::1".parse().unwrap()),
        ));

        let mut ctx = crate::plugin::Context::new(request);
        ctx.set_response(Some(response));
        plugin.execute(&mut ctx).await.unwrap();

        let response = ctx.response().unwrap();
        // Both records should be preserved when query type is AAAA
        assert_eq!(response.answers().len(), 2);
    }
}
