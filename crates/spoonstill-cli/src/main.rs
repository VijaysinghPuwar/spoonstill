//! `still` — the spoonstill command line.
//!
//! D-010: this is the permanent, complete control surface, not a stepping stone
//! to the GUI. **If the CLI cannot do it, it does not exist.** The Tauri shell
//! arriving at M4 is an adapter over `spoonstill-app` and owns no business
//! logic, so every capability lands here first.
//!
//! M0 ships the binary and nothing behind it. `render-scene` arrives at M1.

fn main() {
    // M0 is the skeleton milestone: this proves the binary builds, links the
    // application layer, and is named `still`. Argument parsing arrives with
    // the first real subcommand at M1, so that the CLI surface is designed
    // once, against real commands, rather than grown from a placeholder.
    println!(
        "{} {} — batch (still + narration) -> Ken Burns MP4",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION")
    );
    println!("state dir: {}", spoonstill_core::STATE_DIR);
    println!("manifest:  {}", spoonstill_core::MANIFEST_FILE);
    println!();
    println!("No subcommands yet — M0 is the workspace skeleton.");
    println!("Next: `still render-scene --image X.jpg --audio Y.mp3 --out seg.mp4` (M1).");
}
