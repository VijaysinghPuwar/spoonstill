//! What a reorder costs, stated as a fact rather than as a suspicion (D-140),
//! and what the versioned seed does about it (D-153).

use spoonstill_core::MotionSeed;
use spoonstill_core::motion::MotionSpec;

/// D-140's original finding, kept: under `v1` a photograph's move is a function
/// of where it sits, so moving one scene re-rolls the motion on every scene.
#[test]
fn moving_a_photo_changes_the_move_it_is_given() {
    let content = "same-photo-same-bytes";
    let mut descriptors = Vec::new();
    for index in 0..6 {
        descriptors.push(MotionSpec::seeded("trip", index, content).descriptor());
    }
    println!("one photo, six positions:");
    for (i, d) in descriptors.iter().enumerate() {
        println!("  index {i}: {d}");
    }
    let distinct: std::collections::HashSet<_> = descriptors.iter().collect();
    assert!(
        distinct.len() > 1,
        "the same photo gets the same move at every position, so this test is moot"
    );
}

/// D-153, the whole point: under `v2` the move is a property of the photograph.
/// Six positions, one move — so a reorder re-encodes nothing and changes the
/// motion on nobody.
#[test]
fn under_the_new_rule_a_photo_moves_the_same_way_wherever_it_sits() {
    let content = "same-photo-same-bytes";
    let moves: std::collections::HashSet<String> = (0..6)
        .map(|index| {
            MotionSpec::seeded_with(MotionSeed::V2, "trip", index, 0, content).descriptor()
        })
        .collect();
    assert_eq!(
        moves.len(),
        1,
        "the move still depends on the position: {moves:?}"
    );
}

/// And the property `v1` used the scene index to get: one photograph shown
/// twice in one film does not move identically both times. `occurrence` is
/// what carries that, and unlike the index it does not change when an
/// unrelated scene moves past.
#[test]
fn one_photo_shown_twice_still_gets_two_different_moves() {
    let content = "a-photograph-used-in-two-scenes";
    let first = MotionSpec::seeded_with(MotionSeed::V2, "trip", 0, 0, content).descriptor();
    let second = MotionSpec::seeded_with(MotionSeed::V2, "trip", 9, 1, content).descriptor();
    assert_ne!(
        first, second,
        "the repeat is an exact copy of the first move"
    );
}

/// The promise the version exists to keep: `v1` is frozen. These are the moves
/// every film this program has ever made, and they must not drift — a value
/// here changing means somebody's re-render no longer matches what they made.
#[test]
fn the_old_rule_still_produces_exactly_the_moves_it_always_did() {
    let content = "same-photo-same-bytes";
    let under_v1: Vec<String> = (0..6)
        .map(|index| MotionSpec::seeded("trip", index, content).descriptor())
        .collect();
    assert_eq!(
        under_v1,
        vec![
            "pan-up@south-east:0.0800",
            "pan-right@east:0.0800",
            "pan-up@center:0.0800",
            "pan-right@north-east:0.1100",
            "zoom-in@south-west:0.1200",
            "pan-down@south:0.0800",
        ],
        "D-140 recorded these six; a change here is a change to films already made"
    );

    // And asking for v1 by name is the same thing as the frozen function.
    for index in 0..6 {
        assert_eq!(
            MotionSpec::seeded_with(MotionSeed::V1, "trip", index, 7, content),
            MotionSpec::seeded("trip", index, content),
            "v1 must ignore the occurrence entirely"
        );
    }
}

/// A `v2` seed must not collide with a `v1` seed, or one film in a set would
/// silently be unchanged while the rest moved — which reads as a bug whichever
/// way it is noticed.
#[test]
fn the_two_rules_do_not_agree_by_accident() {
    let mut same = 0;
    for n in 0..500 {
        let content = format!("photo-{n}");
        if MotionSpec::seeded_with(MotionSeed::V2, "trip", 0, 0, &content)
            == MotionSpec::seeded(MotionSeed::V1.as_str(), 0, &content)
        {
            same += 1;
        }
    }
    // Not "never": two rules over a five-value amount, six kinds and nine
    // anchors will agree by chance about one time in 270. What must not happen
    // is the *seed* being identical, which is what tagging v2 prevents.
    assert!(
        same < 20,
        "{same} of 500 agreed — the rules are not independent"
    );

    let v1 = MotionSpec::seeded("trip", 0, "photo");
    let v2 = MotionSpec::seeded_with(MotionSeed::V2, "trip", 0, 0, "photo");
    assert_ne!(
        v1.seed, v2.seed,
        "v2 hashes the same bytes as v1 at index 0"
    );
}

/// The spelling an operator types, and the one they mistype.
#[test]
fn the_setting_is_parsed_the_way_a_person_writes_it() {
    for text in ["v1", "V1", " 1 ", "1"] {
        assert_eq!(MotionSeed::parse(text), Some(MotionSeed::V1), "{text:?}");
    }
    for text in ["v2", "V2", "2"] {
        assert_eq!(MotionSeed::parse(text), Some(MotionSeed::V2), "{text:?}");
    }
    for text in ["v3", "", "latest", "2.0"] {
        assert_eq!(MotionSeed::parse(text), None, "{text:?}");
    }
    assert_eq!(
        MotionSeed::default(),
        MotionSeed::V1,
        "absent means the old rule"
    );
}
