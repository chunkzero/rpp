wit_bindgen::generate!({
    world: "process-test",
    path: "wit",
    with: {
        "rpp:host/process@0.1.0": generate,
    },
});

use rpp::host::process::{run, Request};

struct Component;

impl Guest for Component {
    fn run_echo(program: String) -> Result<String, String> {
        let output = run(&Request {
            program,
            args: vec!["hi".into()],
            cwd: None,
            environment: Vec::new(),
            stdin: Vec::new(),
            timeout_ms: Some(1_000),
        })?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

export!(Component);
