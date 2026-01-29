#[cfg(feature = "lua")]
#[test]
fn test_lua_runtime_basic() {
    use rpp::lua::{LuaProcessResult, LuaRuntime};
    use rpp::sandbox::SandboxContext;
    use std::path::PathBuf;

    let mut runtime = LuaRuntime::new().expect("Failed to create runtime");

    let plugin_source = r#"
        return {
            name = "test_plugin",
            version = "1.0.0",
            type = "processor",
            patterns = {"*.txt"},
            priority = 100,

            process = function(ctx, input)
                return {
                    action = "continue",
                    content = "modified: " .. input.content
                }
            end
        }
    "#;

    runtime
        .load_plugin("test_plugin", plugin_source)
        .expect("Failed to load plugin");

    let sandbox = SandboxContext::new(PathBuf::from("/test"), PathBuf::from("/output"));
    let content = b"hello world";

    let result = runtime
        .call_processor("test_plugin", "test.txt", content, &sandbox)
        .expect("Failed to call processor");

    match result {
        LuaProcessResult::Continue { content, .. } => {
            assert_eq!(content, b"modified: hello world");
        }
        _ => panic!("Expected Continue result"),
    }
}

#[cfg(feature = "lua")]
#[test]
fn test_lua_runtime_json_api() {
    use rpp::lua::{LuaProcessResult, LuaRuntime};
    use rpp::sandbox::SandboxContext;
    use std::path::PathBuf;

    let mut runtime = LuaRuntime::new().expect("Failed to create runtime");

    let plugin_source = r#"
        return {
            name = "json_test",
            version = "1.0.0",
            type = "processor",
            patterns = {"*.json"},
            priority = 100,

            process = function(ctx, input)
                local data = ctx.json.decode(input.content)
                data.modified = true
                return {
                    action = "continue",
                    content = ctx.json.encode_compact(data)
                }
            end
        }
    "#;

    runtime
        .load_plugin("json_test", plugin_source)
        .expect("Failed to load plugin");

    let sandbox = SandboxContext::new(PathBuf::from("/test"), PathBuf::from("/output"));
    let content = br#"{"name":"test","value":42}"#;

    let result = runtime
        .call_processor("json_test", "test.json", content, &sandbox)
        .expect("Failed to call processor");

    match result {
        LuaProcessResult::Continue { content, .. } => {
            let text = String::from_utf8(content).expect("Invalid UTF-8");
            assert!(text.contains("\"modified\":true"));
        }
        _ => panic!("Expected Continue result"),
    }
}

#[cfg(feature = "lua")]
#[test]
fn test_lua_runtime_hash_api() {
    use rpp::lua::{LuaProcessResult, LuaRuntime};
    use rpp::sandbox::SandboxContext;
    use std::path::PathBuf;

    let mut runtime = LuaRuntime::new().expect("Failed to create runtime");

    let plugin_source = r#"
        return {
            name = "hash_test",
            version = "1.0.0",
            type = "processor",
            patterns = {"*.txt"},
            priority = 100,

            process = function(ctx, input)
                local hash = ctx.hash.xxhash3(input.content)
                local sha = ctx.hash.sha256(input.content)
                local md5_hash = ctx.hash.md5(input.content)

                return {
                    action = "continue",
                    content = string.format("xxhash3=%s\nsha256=%s\nmd5=%s", hash, sha, md5_hash)
                }
            end
        }
    "#;

    runtime
        .load_plugin("hash_test", plugin_source)
        .expect("Failed to load plugin");

    let sandbox = SandboxContext::new(PathBuf::from("/test"), PathBuf::from("/output"));
    let content = b"test data";

    let result = runtime
        .call_processor("hash_test", "test.txt", content, &sandbox)
        .expect("Failed to call processor");

    match result {
        LuaProcessResult::Continue { content, .. } => {
            let text = String::from_utf8(content).expect("Invalid UTF-8");
            assert!(text.contains("xxhash3="));
            assert!(text.contains("sha256="));
            assert!(text.contains("md5="));
        }
        _ => panic!("Expected Continue result"),
    }
}

#[cfg(feature = "lua")]
#[test]
fn test_lua_processor() {
    use rpp::plugin::{LuaProcessor, Plugin, ProcessResult, ProcessingContext, ProcessorPlugin};
    use std::path::Path;

    let plugin_source = r#"
        return {
            name = "uppercase",
            version = "1.0.0",
            type = "processor",
            patterns = {"*.txt"},
            priority = 100,

            process = function(ctx, input)
                return {
                    action = "continue",
                    content = string.upper(input.content)
                }
            end
        }
    "#;

    let processor = LuaProcessor::new(
        "uppercase".to_string(),
        "1.0.0".to_string(),
        vec!["*.txt".to_string()],
        100,
        plugin_source.to_string(),
    );

    assert_eq!(processor.name(), "uppercase");
    assert_eq!(processor.version(), "1.0.0");
    assert_eq!(processor.patterns(), &["*.txt"]);
    assert_eq!(processor.priority(), 100);

    let content = b"hello world";
    let config = toml::Value::Table(toml::map::Map::new());
    let ctx = ProcessingContext {
        path: Path::new("test.txt"),
        content,
        source_path: Path::new("/test/test.txt"),
        config: &config,
    };

    let result = processor.process(&ctx).expect("Failed to process");

    match result {
        ProcessResult::Continue { content, .. } => {
            assert_eq!(content, b"HELLO WORLD");
        }
        _ => panic!("Expected Continue result"),
    }
}

#[cfg(feature = "lua")]
#[test]
fn test_lua_chain_processing() {
    use rpp::lua::LuaRuntime;

    let mut runtime = LuaRuntime::new().expect("Failed to create runtime");

    let plugin1 = r#"
        return {
            process = function(ctx, input)
                return {
                    action = "continue",
                    content = input.content .. "-p1",
                    path = input.path
                }
            end
        }
    "#;

    let plugin2 = r#"
        return {
            process = function(ctx, input)
                return {
                    action = "continue",
                    content = input.content .. "-p2",
                    path = input.path
                }
            end
        }
    "#;

    runtime
        .load_plugin_versioned("p1", "1.0.0", plugin1)
        .expect("Failed to load plugin1");
    runtime
        .load_plugin_versioned("p2", "1.0.0", plugin2)
        .expect("Failed to load plugin2");

    let result = runtime
        .process_chain(
            "test.txt",
            b"data",
            &[
                ("p1".to_string(), "1.0.0".to_string()),
                ("p2".to_string(), "1.0.0".to_string()),
            ],
        )
        .expect("Failed to process chain");

    assert!(!result.cancelled);
    assert_eq!(result.skipped_at, None);
    assert_eq!(result.content, b"data-p1-p2");
    assert_eq!(result.transformations.len(), 2);
    assert_eq!(result.transformations[0].0, "p1");
    assert_eq!(result.transformations[1].0, "p2");
}

#[cfg(feature = "lua")]
#[test]
fn test_plugin_isolation() {
    use rpp::lua::LuaRuntime;

    let mut runtime = LuaRuntime::new().expect("Failed to create runtime");

    // Plugin 1 sets a variable in its own environment
    let plugin1 = r#"
        shared_value = "plugin1"
        return {
            process = function(ctx, input)
                return {
                    action = "continue",
                    content = shared_value or "nil",
                    path = input.path
                }
            end
        }
    "#;

    // Plugin 2 tries to access plugin1's variable (should fail)
    let plugin2 = r#"
        return {
            process = function(ctx, input)
                -- shared_value should be nil in this plugin's environment
                local status = (shared_value == nil) and "isolated" or "leaked"
                return {
                    action = "continue",
                    content = status,
                    path = input.path
                }
            end
        }
    "#;

    runtime
        .load_plugin_versioned("p1", "1.0.0", plugin1)
        .expect("Failed to load plugin1");
    runtime
        .load_plugin_versioned("p2", "1.0.0", plugin2)
        .expect("Failed to load plugin2");

    // First verify plugin1 can see its own variable
    let result1 = runtime
        .process_chain("test.txt", b"data", &[("p1".to_string(), "1.0.0".to_string())])
        .expect("Failed to process p1");
    assert_eq!(result1.content, b"plugin1");

    // Then verify plugin2 cannot see plugin1's variable
    let result2 = runtime
        .process_chain("test.txt", b"data", &[("p2".to_string(), "1.0.0".to_string())])
        .expect("Failed to process p2");
    assert_eq!(result2.content, b"isolated");
}

#[cfg(feature = "lua")]
#[test]
fn test_versioned_plugin_loading() {
    use rpp::lua::LuaRuntime;

    let mut runtime = LuaRuntime::new().expect("Failed to create runtime");

    let plugin_v1 = r#"
        return {
            process = function(ctx, input)
                return {
                    action = "continue",
                    content = "v1",
                    path = input.path
                }
            end
        }
    "#;

    let plugin_v2 = r#"
        return {
            process = function(ctx, input)
                return {
                    action = "continue",
                    content = "v2",
                    path = input.path
                }
            end
        }
    "#;

    runtime
        .load_plugin_versioned("test", "1.0.0", plugin_v1)
        .expect("Failed to load v1");
    runtime
        .load_plugin_versioned("test", "2.0.0", plugin_v2)
        .expect("Failed to load v2");

    assert!(runtime.has_plugin("test", "1.0.0"));
    assert!(runtime.has_plugin("test", "2.0.0"));

    // Process with v1
    let result1 = runtime
        .process_chain("test.txt", b"data", &[("test".to_string(), "1.0.0".to_string())])
        .expect("Failed with v1");
    assert_eq!(result1.content, b"v1");

    // Process with v2
    let result2 = runtime
        .process_chain("test.txt", b"data", &[("test".to_string(), "2.0.0".to_string())])
        .expect("Failed with v2");
    assert_eq!(result2.content, b"v2");
}

#[cfg(feature = "lua")]
#[test]
fn test_chain_skip_action() {
    use rpp::lua::LuaRuntime;

    let mut runtime = LuaRuntime::new().expect("Failed to create runtime");

    let plugin1 = r#"
        return {
            process = function(ctx, input)
                return {
                    action = "continue",
                    content = input.content .. "-p1",
                    path = input.path
                }
            end
        }
    "#;

    let plugin2 = r#"
        return {
            process = function(ctx, input)
                return {
                    action = "skip"
                }
            end
        }
    "#;

    let plugin3 = r#"
        return {
            process = function(ctx, input)
                return {
                    action = "continue",
                    content = input.content .. "-p3",
                    path = input.path
                }
            end
        }
    "#;

    runtime
        .load_plugin_versioned("p1", "1.0.0", plugin1)
        .expect("Failed to load plugin1");
    runtime
        .load_plugin_versioned("p2", "1.0.0", plugin2)
        .expect("Failed to load plugin2");
    runtime
        .load_plugin_versioned("p3", "1.0.0", plugin3)
        .expect("Failed to load plugin3");

    let result = runtime
        .process_chain(
            "test.txt",
            b"data",
            &[
                ("p1".to_string(), "1.0.0".to_string()),
                ("p2".to_string(), "1.0.0".to_string()),
                ("p3".to_string(), "1.0.0".to_string()),
            ],
        )
        .expect("Failed to process chain");

    assert_eq!(result.skipped_at, Some(1)); // Skipped at plugin2
    assert_eq!(result.content, b"data-p1"); // Only p1 processed
    assert_eq!(result.transformations.len(), 1); // Only p1 recorded
}

#[cfg(feature = "lua")]
#[test]
fn test_chain_cancel_action() {
    use rpp::lua::LuaRuntime;

    let mut runtime = LuaRuntime::new().expect("Failed to create runtime");

    let plugin1 = r#"
        return {
            process = function(ctx, input)
                return {
                    action = "continue",
                    content = input.content .. "-p1",
                    path = input.path
                }
            end
        }
    "#;

    let plugin2 = r#"
        return {
            process = function(ctx, input)
                return {
                    action = "cancel"
                }
            end
        }
    "#;

    runtime
        .load_plugin_versioned("p1", "1.0.0", plugin1)
        .expect("Failed to load plugin1");
    runtime
        .load_plugin_versioned("p2", "1.0.0", plugin2)
        .expect("Failed to load plugin2");

    let result = runtime
        .process_chain(
            "test.txt",
            b"data",
            &[
                ("p1".to_string(), "1.0.0".to_string()),
                ("p2".to_string(), "1.0.0".to_string()),
            ],
        )
        .expect("Failed to process chain");

    assert!(result.cancelled);
    assert_eq!(result.output_path, None);
}
