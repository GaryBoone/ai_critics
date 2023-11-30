use color_eyre::config::HookBuilder;
use color_eyre::eyre::Report;

// Set up color-eyre to show backtraces, but only for this crate.
pub fn setup_color_eyre() -> Result<(), Report> {
    std::env::set_var("RUST_BACKTRACE", "full"); // Enable backtraces

    HookBuilder::default()
        .add_frame_filter(Box::new(|frames| {
            let filters = &["ai_critics::"]; // Replace with your crate's name
            frames.retain(|frame| {
                if let Some(name) = &frame.name {
                    filters
                        .iter()
                        .any(|filter| name.as_str().starts_with(filter))
                } else {
                    false
                }
            });
        }))
        .install()?;

    Ok(())
}
