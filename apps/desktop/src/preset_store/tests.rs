//! Real temporary filesystem checks; no user profile or music files are touched.

use super::*;
use resolibre_core::equalizer::{Equalizer, preset::Preset};
use std::fs;
use std::time::{Duration, Instant};

/// Creates an isolated fixture root unique to this test.
fn directory(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("resolibre-presets-{name}-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    directory
}

/// Waits only in tests, bounding owned background operations.
fn wait(store: &mut PresetStore) -> Completion {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(completion) = store.poll().unwrap() {
            return completion;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Named saves, explicit replacement, reload and portable transfers preserve confirmed values.
#[test]
fn named_presets_round_trip_and_replace_explicitly() {
    let root = directory("round-trip");
    let profile = root.join("profile");
    let mut store = PresetStore::start(profile.clone()).unwrap();
    wait(&mut store);
    for index in 0..5 {
        let preset = Preset::new(
            &format!("Headphones {index}"),
            Equalizer::preset(index % 4).unwrap(),
        )
        .unwrap();
        store.save(preset, false).unwrap();
        wait(&mut store);
    }
    assert_eq!(store.entries.len(), 5, "not limited to three slots");
    let previous = store.entries[0].preset.clone();
    let replacement = Preset::new(
        &previous.name,
        Equalizer::new(false, [-3.0; 10], -1.5).unwrap(),
    )
    .unwrap();
    assert!(files::save(&profile, replacement.clone(), false).is_err());
    let target = profile.join("eq-presets").join(&store.entries[0].file);
    assert_eq!(files::read_preset(&target).unwrap(), previous);
    store.save(replacement.clone(), true).unwrap();
    wait(&mut store);
    assert_eq!(store.entries.len(), 5);
    let exported = root.join("shared.resolibre-eq");
    store.export(exported.clone(), replacement.clone()).unwrap();
    assert!(matches!(wait(&mut store), Completion::Exported));
    let bytes = fs::read(&exported).unwrap();
    assert!(files::export(&exported, &previous).is_err());
    assert_eq!(fs::read(&exported).unwrap(), bytes);
    store.import(exported.clone()).unwrap();
    let Completion::Imported(imported) = wait(&mut store) else {
        panic!("import completion");
    };
    assert_eq!(imported, replacement);
    drop(store);
    let mut reloaded = PresetStore::start(profile).unwrap();
    wait(&mut reloaded);
    assert_eq!(reloaded.entries[0].preset, replacement);
    assert_eq!(reloaded.entries.len(), 5);
    drop(reloaded);
    fs::remove_dir_all(root).unwrap();
}

/// Legacy slots migrate once; malformed/new symlinks, special files and failed writes stay isolated.
#[test]
fn migration_and_untrusted_files_preserve_good_presets() {
    let root = directory("migration");
    let legacy = b"ReSolibre-EQ-1\n1 -24 0 1 2 3 4 5 6 7 8 9\n-\n0 12 0 0 0 0 0 0 0 0 0 0\n";
    fs::write(root.join("equalizer-v1.txt"), legacy).unwrap();
    let Completion::Catalog { entries, .. } = files::load(&root).unwrap() else {
        panic!()
    };
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].preset.name, "Saved 1");
    assert_eq!(entries[0].preset.settings.preamp(), -2.0);
    assert_eq!(entries[1].preset.settings.preamp(), 2.0);
    let folder = root.join("eq-presets");
    let replacement = Preset::new("Saved 1", Equalizer::preset(2).unwrap()).unwrap();
    files::save(&root, replacement.clone(), true).unwrap();
    files::load(&root).unwrap();
    assert_eq!(
        files::read_preset(&folder.join("legacy-1.resolibre-eq")).unwrap(),
        replacement
    );
    assert_eq!(fs::read(root.join("equalizer-v1.txt")).unwrap(), legacy);
    fs::write(folder.join("bad.resolibre-eq"), vec![0; 4097]).unwrap();
    std::os::unix::fs::symlink(root.join("equalizer-v1.txt"), folder.join("link.resolibre-eq"))
        .unwrap();
    assert!(files::read_preset(&folder.join("link.resolibre-eq")).is_err());
    assert!(
        std::process::Command::new("mkfifo")
            .arg(folder.join("pipe.resolibre-eq"))
            .status()
            .unwrap()
            .success()
    );
    assert!(files::read_preset(&folder.join("pipe.resolibre-eq")).is_err());
    let Completion::Catalog {
        entries, skipped, ..
    } = files::load(&root).unwrap()
    else {
        panic!()
    };
    assert!(skipped);
    assert_eq!(entries.len(), 2);
    let before = fs::read(folder.join("legacy-1.resolibre-eq")).unwrap();
    fs::write(
        folder.join(format!(".legacy-1.resolibre-eq-{}.tmp", std::process::id())),
        b"occupied",
    )
    .unwrap();
    assert!(files::save(&root, replacement, true).is_err());
    assert_eq!(fs::read(folder.join("legacy-1.resolibre-eq")).unwrap(), before);
    let music = root.join("song.flac");
    fs::write(&music, b"original music").unwrap();
    assert!(files::export(&music, &entries[0].preset).is_err());
    assert_eq!(fs::read(music).unwrap(), b"original music");
    fs::remove_dir_all(root).unwrap();
}

/// Untrusted labels remain labels, and catalog limits reject extra saves without removing files.
#[test]
fn names_are_not_paths_and_catalog_is_bounded() {
    let root = directory("bounds");
    files::load(&root).unwrap();
    let preset = Preset::new("../../outside", Equalizer::default()).unwrap();
    files::save(&root, preset, false).unwrap();
    let folder = root.join("eq-presets");
    for index in 1..MAX_PRESETS {
        let preset = Preset::new(&format!("Preset {index}"), Equalizer::default()).unwrap();
        fs::write(
            folder.join(format!("fixture-{index}.resolibre-eq")),
            preset.encode(),
        )
        .unwrap();
    }
    assert!(
        files::save(
            &root,
            Preset::new("Overflow", Equalizer::default()).unwrap(),
            false
        )
        .is_err()
    );
    let Completion::Catalog { entries, .. } = files::load(&root).unwrap() else {
        panic!()
    };
    assert_eq!(entries.len(), MAX_PRESETS);
    assert!(folder.join("preset-1.resolibre-eq").is_file());
    assert!(!root.join("outside").exists());
    fs::remove_dir_all(root).unwrap();
}
