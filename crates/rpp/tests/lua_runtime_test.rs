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
