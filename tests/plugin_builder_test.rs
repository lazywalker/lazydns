//! Integration tests for the plugin builder system

use lazydns::config::types::PluginConfig;
use lazydns::plugin::factory;
use lazydns::plugin::factory::init;
use serde_yaml::{Mapping, Value};

#[test]
fn test_create_cache_plugin_from_builder() {
    init();

    let mut args = Mapping::new();
    args.insert(Value::String("size".into()), Value::Number(2048.into()));

    let config = PluginConfig {
        tag: Some("test_cache".to_string()),
        plugin_type: "cache".to_string(),
        args: Value::Mapping(args),
    };

    let builder_obj =
        factory::get_plugin_factory("cache").expect("cache builder should be registered");
    let plugin = builder_obj
        .create(&config)
        .expect("plugin creation should succeed");

    assert_eq!(plugin.name(), "cache");
}

#[test]
fn test_create_forward_plugin_from_builder() {
    init();

    let upstreams = vec![
        Value::String("8.8.8.8:53".to_string()),
        Value::String("8.8.4.4:53".to_string()),
    ];
    let mut args = Mapping::new();
    args.insert(
        Value::String("upstreams".into()),
        Value::Sequence(upstreams),
    );

    let config = PluginConfig {
        tag: Some("test_forward".to_string()),
        plugin_type: "forward".to_string(),
        args: Value::Mapping(args),
    };

    let builder_obj =
        factory::get_plugin_factory("forward").expect("forward builder should be registered");
    let plugin = builder_obj
        .create(&config)
        .expect("plugin creation should succeed");

    assert_eq!(plugin.name(), "forward");
}

#[test]
fn test_create_query_acl_plugin_from_builder() {
    init();

    let rule = {
        let mut rule = Mapping::new();
        rule.insert(
            Value::String("network".into()),
            Value::String("10.0.0.0/8".to_string()),
        );
        rule.insert(
            Value::String("action".into()),
            Value::String("allow".to_string()),
        );
        rule
    };

    let mut args = Mapping::new();
    args.insert(
        Value::String("rules".into()),
        Value::Sequence(vec![Value::Mapping(rule)]),
    );

    let config = PluginConfig {
        tag: Some("test_acl".to_string()),
        plugin_type: "query_acl".to_string(),
        args: Value::Mapping(args),
    };

    let builder_obj =
        factory::get_plugin_factory("query_acl").expect("query_acl builder should be registered");
    let plugin = builder_obj
        .create(&config)
        .expect("plugin creation should succeed");

    assert_eq!(plugin.name(), "query_acl");
}
