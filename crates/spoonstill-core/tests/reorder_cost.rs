//! What a reorder costs, stated as a fact rather than as a suspicion (D-140).
use spoonstill_core::motion::MotionSpec;

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
