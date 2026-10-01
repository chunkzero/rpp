pub(crate) fn web() -> Vec<deno_core::Extension> {
    vec![
        deno_webidl::deno_webidl::init(),
        deno_web::deno_web::init(
            deno_web::BlobStore::default_arc(),
            None,
            false,
            deno_web::InMemoryBroadcastChannel::default(),
        ),
    ]
}
