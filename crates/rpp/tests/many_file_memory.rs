//! Manual memory fixture: run the ignored executable with an OS peak-RSS measurement tool.

use rpp::{config::Config, engine::Engine};

#[test]
#[ignore = "manual many-file peak RSS measurement"]
fn many_file_memory() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("src");
    std::fs::create_dir(&source).unwrap();
    for index in 0..256u32 {
        let mut bytes = vec![b'x'; 1024 * 1024];
        bytes[..4].copy_from_slice(&index.to_le_bytes());
        std::fs::write(source.join(format!("{index:04}.bin")), bytes).unwrap();
    }
    let mut config = Config::new("memory", 34);
    config.build.workers = 4;
    let engine = Engine::builder(config)
        .project_root(dir.path())
        .build_engine()
        .unwrap();
    assert_eq!(engine.build().unwrap().processed, 256);
    assert_eq!(engine.build().unwrap().cached, 256);
    for index in 0..256u32 {
        let bytes = std::fs::read(dir.path().join(format!("dist/{index:04}.bin"))).unwrap();
        assert_eq!(bytes.len(), 1024 * 1024);
        assert_eq!(&bytes[..4], &index.to_le_bytes());
    }
}
