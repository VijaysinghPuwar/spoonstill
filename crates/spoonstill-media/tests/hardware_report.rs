//! The hardware report is checked against the hardware (D-159).
//!
//! `hardware::detect` claims an encoder works by running it. This asserts that
//! the claim is true **in both directions**, against an FFmpeg invocation this
//! file builds itself rather than through the module under test:
//!
//! - every encoder reported usable really does encode here, and
//! - every encoder reported unusable really does fail here.
//!
//! The second half is the one that matters, and it is what a `-encoders` grep
//! could never pass. On the machine this was written on, `ffmpeg -encoders`
//! lists `h264_qsv` and `h264_qsv` cannot open a session, so a detector that
//! trusted the listing would report a usable Intel encoder on a machine with an
//! AMD processor. That is the defect this test exists to keep out, and it fails
//! against a `detect` rewritten to return the listing.
//!
//! On a machine with no FFmpeg every candidate is `Absent` and there is nothing
//! to cross-check. That is honestly vacuous rather than quietly so: the test
//! says which case it ran in, and `still doctor` is the surface that reports a
//! missing FFmpeg (D-103).

use spoonstill_media::Tools;
use spoonstill_media::hardware::{Support, candidates, detect};

/// Encode a few software frames with `encoder`, without going through the
/// module under test. Deliberately a second implementation: a cross-check that
/// shares its subject's code checks nothing.
fn encodes_here(tools: &Tools, encoder: &str) -> bool {
    std::process::Command::new(tools.ffmpeg())
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=320x240:r=25:d=0.2",
            "-pix_fmt",
            "yuv420p",
            "-c:v",
            encoder,
            "-f",
            "null",
            "-",
        ])
        .output()
        .is_ok_and(|out| out.status.success())
}

#[test]
fn every_claim_in_the_report_is_true_of_this_machine() {
    let tools = Tools::from_env();
    if tools.ready().is_err() {
        eprintln!("no FFmpeg on this machine — nothing to cross-check");
        return;
    }

    let report = detect(&tools);
    assert_eq!(report.len(), candidates().len());

    let mut checked = 0;
    for one in &report {
        let encoder = one.candidate.encoder;
        match &one.support {
            Support::Usable => {
                checked += 1;
                assert!(
                    encodes_here(&tools, encoder),
                    "{encoder} was reported usable and does not encode here"
                );
            }
            Support::Unusable { reason } => {
                checked += 1;
                assert!(
                    !encodes_here(&tools, encoder),
                    "{encoder} was reported unusable ({reason}) and encodes here fine"
                );
            }
            // Not in this build. There is nothing to run and nothing to claim.
            Support::Absent => {}
        }
        eprintln!("  {encoder:<20} {:?}", one.support);
    }

    eprintln!(
        "{checked} of {} candidates were in this build",
        report.len()
    );
}
