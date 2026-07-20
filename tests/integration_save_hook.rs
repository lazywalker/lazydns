use std::sync::Arc;

#[tokio::test]
async fn integration_sequence_save_hook() {
    use lazydns::config::Config;
    use lazydns::dns::types::{RecordClass, RecordType};
    use lazydns::dns::{Message, Question, RData, ResourceRecord};
    use lazydns::plugin::Context;
    use lazydns::plugin::PluginBuilder;
    use lazydns::plugins::executable::ReverseLookupPlugin;
    use lazydns::plugins::{ArbitraryPlugin, SequencePlugin, SequenceStep};

    // Use examples/etc as working directory and load its config
    std::env::set_current_dir("examples/etc").expect("chdir examples/etc");
    let cfg = Config::from_file("config.yaml").expect("load config");

    // Build plugins from config
    let mut builder = PluginBuilder::new();
    for p in &cfg.plugins {
        println!(
            "Building plugin: {} (type: {})",
            p.effective_name(),
            p.plugin_type
        );
        if let Err(e) = builder.build(p) {
            println!("Skipping plugin {}: {}", p.effective_name(), e);
            continue;
        }
    }
    builder
        .resolve_references(&cfg.plugins)
        .expect("resolve refs");

    // Get registry and register a ReverseLookup instance for testing
    let mut registry = builder.get_registry();
    let rl = Arc::new(ReverseLookupPlugin::quick_setup("64"));
    registry.register_replace_with_name("reverse_lookup", rl.clone());

    // Create an arbitrary response message that contains an A answer for example.com
    let mut resp = Message::new();
    resp.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));
    resp.add_answer(ResourceRecord::new(
        "example.com",
        RecordType::A,
        RecordClass::IN,
        300,
        RData::A("192.0.2.5".parse().unwrap()),
    ));

    // Build an ArbitraryPlugin that will set the response when executed
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

    // Sequence with single exec of arbitrary plugin
    let seq = Arc::new(SequencePlugin::with_steps(vec![SequenceStep::Exec(arb)]));
    registry.register_replace_with_name("it_sequence", seq.clone());

    // Execute the sequence via the plugin interface. The request must carry
    // the question ArbitraryPlugin matches against (example.com A); otherwise
    // no response is produced and the save-hook has nothing to cache.
    let mut req = Message::new();
    req.add_question(Question::new("example.com", RecordType::A, RecordClass::IN));
    let mut ctx = Context::new(req);
    let plugin = registry.get("it_sequence").expect("sequence present");
    plugin.execute(&mut ctx).await.expect("execute sequence");

    // After sequence execution, call save_ips_after on reverse_lookup plugins
    if ctx.has_response() {
        let resp_ref = ctx.response().unwrap();
        for name in registry.plugin_names() {
            if let Some(p) = registry.get(&name)
                && p.name() == "reverse_lookup"
                && let Some(rldown) = p.as_ref().as_any().downcast_ref::<ReverseLookupPlugin>()
            {
                rldown.save_ips_after(ctx.request(), resp_ref);
            }
        }
    }

    // Verify the reverse lookup cache contains mapping for the IP
    let ip = std::net::IpAddr::V4("192.0.2.5".parse().unwrap());
    let got = rl.lookup_cached(&ip).expect("expected cached entry");
    assert_eq!(got, "example.com");
}
