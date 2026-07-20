use std::sync::Arc;

#[tokio::test]
async fn integration_ipset_sequence() {
    use lazydns::dns::types::{RecordClass, RecordType};
    use lazydns::dns::{Message, Question, RData, ResourceRecord};
    use lazydns::plugin::Context;
    use lazydns::plugin::Plugin;
    use lazydns::plugins::executable::ipset::{IpSetArgs, IpSetPlugin};
    use lazydns::plugins::{ArbitraryPlugin, SequencePlugin, SequenceStep};

    // no cwd changes required — tests use constructed messages

    // IpSet plugin configured to add to "myset"
    let args = IpSetArgs {
        set_name4: Some("myset".into()),
        set_name6: None,
        mask4: Some(24),
        mask6: None,
    };
    let ipset = Arc::new(IpSetPlugin::new(args));

    // Response containing an A record
    let mut resp = Message::new();
    resp.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));
    resp.add_answer(ResourceRecord::new(
        "example.com",
        RecordType::A,
        RecordClass::IN,
        300,
        RData::A("203.0.113.7".parse().unwrap()),
    ));

    // Construct rule-based ArbitraryPlugin from the response's answers
    use lazydns::plugins::dataset::arbitrary::ArbitraryArgs;
    let mut rules = Vec::new();
    for rr in resp.answers() {
        match rr.rdata() {
            RData::A(ip) => rules.push(format!("{} A {}", rr.name(), ip)),
            RData::AAAA(ip) => rules.push(format!("{} AAAA {}", rr.name(), ip)),
            _ => {}
        }
    }
    let args = ArbitraryArgs {
        rules: Some(rules),
        files: None,
    };
    let arb = Arc::new(ArbitraryPlugin::new(args).unwrap());

    let arb_plugin: Arc<dyn lazydns::plugin::Plugin> = arb;
    let ipset_plugin: Arc<dyn lazydns::plugin::Plugin> = ipset;

    let seq = SequencePlugin::with_steps(vec![
        SequenceStep::Exec(arb_plugin.clone()),
        SequenceStep::Exec(ipset_plugin.clone()),
    ]);

    let mut req = Message::new();
    // The request must carry the question ArbitraryPlugin matches against;
    // otherwise no response is produced and ipset has nothing to add.
    req.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));
    let mut ctx = Context::new(req);
    // bring Plugin trait into scope so execute is available
    seq.execute(&mut ctx).await.expect("execute sequence");

    let added = ctx
        .get_metadata::<Vec<(String, String)>>("ipset_added")
        .expect("ipset_added metadata");
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].0, "myset");
}

#[tokio::test]
async fn integration_nftset_sequence() {
    use lazydns::dns::types::{RecordClass, RecordType};
    use lazydns::dns::{Message, Question, RData, ResourceRecord};
    use lazydns::plugin::Context;
    use lazydns::plugin::Plugin;
    use lazydns::plugins::executable::nftset::{NftSetArgs, NftSetPlugin, SetArgs};
    use lazydns::plugins::{ArbitraryPlugin, SequencePlugin, SequenceStep};

    // no cwd changes required — tests use constructed messages

    let sa = SetArgs {
        table_family: Some("inet".into()),
        table: Some("t".into()),
        set: Some("s".into()),
        mask: Some(24),
    };
    let args = NftSetArgs {
        ipv4: Some(sa),
        ipv6: None,
    };
    let nft = Arc::new(NftSetPlugin::new(args));

    let mut resp = Message::new();
    resp.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));
    resp.add_answer(ResourceRecord::new(
        "example.com",
        RecordType::A,
        RecordClass::IN,
        300,
        RData::A("198.51.100.9".parse().unwrap()),
    ));

    use lazydns::plugins::dataset::arbitrary::ArbitraryArgs;
    let mut rules = Vec::new();
    for rr in resp.answers() {
        match rr.rdata() {
            RData::A(ip) => rules.push(format!("{} A {}", rr.name(), ip)),
            RData::AAAA(ip) => rules.push(format!("{} AAAA {}", rr.name(), ip)),
            _ => {}
        }
    }
    let args = ArbitraryArgs {
        rules: Some(rules),
        files: None,
    };
    let arb = Arc::new(ArbitraryPlugin::new(args).unwrap());

    let arb_plugin: Arc<dyn lazydns::plugin::Plugin> = arb;
    let nft_plugin: Arc<dyn lazydns::plugin::Plugin> = nft;

    let seq = SequencePlugin::with_steps(vec![
        SequenceStep::Exec(arb_plugin.clone()),
        SequenceStep::Exec(nft_plugin.clone()),
    ]);

    let mut req = Message::new();
    // The request must carry the question ArbitraryPlugin matches against;
    // otherwise no response is produced and nftset has nothing to add.
    req.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));
    let mut ctx = Context::new(req);
    seq.execute(&mut ctx).await.expect("execute sequence");

    let added = ctx
        .get_metadata::<Vec<(String, String)>>("nftset_added_v4")
        .expect("nftset_added_v4 metadata");
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].0, "s");
}
