use std::sync::Once;

pub fn initialize(debug: bool, other_crates: bool) {
    static START: Once = Once::new();

    START.call_once(move || {
        env_logger::Builder::from_default_env()
            .format_timestamp(None)
            .format_module_path(false)
            .filter(
                if other_crates { None } else { Some("gist") },
                if debug {
                    log::LevelFilter::Debug
                } else {
                    log::LevelFilter::Info
                },
            )
            .init();
    });
}
