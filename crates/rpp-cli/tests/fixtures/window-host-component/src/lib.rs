wit_bindgen::generate!({
    world: "window-host",
    path: "wit",
});

struct Component;

impl Guest for Component {
    fn round_trip_options(values: OptionalValues) -> OptionalValues {
        assert_eq!(values.items, vec![None, Some(false), None]);
        assert_eq!(values.pair, (Some(false), None));
        assert_eq!(values.nested, vec![None, Some(None), Some(Some(false))]);
        assert_eq!(values.success, Ok(None));
        assert_eq!(values.failure, Err(None));
        assert_eq!(values.boolean, Ok(false));
        assert_eq!(values.empty, Ok(()));
        values
    }

    fn compile(
        namespace: String,
        project_json: String,
        files: Vec<SourceFile>,
        kotlin_package: Option<String>,
    ) -> Result<CompileOutput, String> {
        if namespace.is_empty() {
            return Err("namespace must not be empty".into());
        }
        if !project_json.contains("fixture") {
            return Err("project JSON did not reach the component".into());
        }
        if !project_json.contains("\"hud_shaders\":true")
            || !project_json.contains("\"pack_format\":84")
        {
            return Err(
                "Window schema v4 requires hud_shaders=true and host pack_format=84".into(),
            );
        }
        if kotlin_package.as_deref() != Some("dev.example.generated") {
            return Err("Window Kotlin package option did not reach the component".into());
        }

        let input = files
            .into_iter()
            .find(|file| file.path == "window/input.bin")
            .ok_or_else(|| "binary source did not reach the component".to_string())?;
        let revision = if cfg!(feature = "v2") { b'2' } else { b'1' };
        let mut pack_contents = vec![b'R', b'P', b'P', revision, 0, 0xff];
        pack_contents.extend(input.contents);

        let (kotlin_path, kotlin_contents) = if cfg!(feature = "v2") {
            (
                "RenamedWindowPack.kt",
                b"// generated schema v4 revision 2\nobject RenamedWindowPack\n".to_vec(),
            )
        } else {
            (
                "WindowPack.kt",
                b"// generated schema v4 revision 1\nobject WindowPack\n".to_vec(),
            )
        };

        Ok(CompileOutput {
            files: vec![OutputFile {
                path: format!("assets/{namespace}/generated.bin"),
                contents: pack_contents,
            }],
            kotlin_files: vec![OutputFile {
                path: kotlin_path.into(),
                contents: kotlin_contents,
            }],
            warnings: vec![format!(
                "compiled Window schema v4 revision {} for pack format 84",
                revision as char
            )],
        })
    }
}

export!(Component);
