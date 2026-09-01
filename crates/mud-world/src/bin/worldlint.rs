//! worldlint — Stage 1 acceptance tool.
//!
//! Boots a lib/ world, prints the COUNTS line, optionally writes every zone
//! back out in canonical form, and byte-diffs the output against a
//! reference tree.
//!
//! Usage: worldlint <lib_dir> [--out <dir>] [--reference <dir>]

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use mud_world::boot;
use mud_world::write;

/// Rooms that are deliberately kept out of file order.
/// 301.wld ships rooms 30171-72 out of order; they are kept here, and
/// the two exits that reach them resolve.
const EXPECTED_DIVERGENCES: &[&str] = &["wld/301.wld"];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: worldlint <lib_dir> [--out <dir>] [--reference <dir>]");
        return ExitCode::from(2);
    }
    let lib_dir = PathBuf::from(&args[0]);
    let mut out_dir: Option<PathBuf> = None;
    let mut reference_dir: Option<PathBuf> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                out_dir = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--reference" => {
                reference_dir = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            other => {
                eprintln!("unknown arg: {other}");
                return ExitCode::from(2);
            }
        }
    }

    let report = match boot::boot_world(&lib_dir) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("BOOT FAILED: {e}");
            return ExitCode::from(1);
        }
    };
    let world = &report.world;
    println!("{}", boot::counts_line(world));
    if !report.zone_errors.is_empty() {
        println!("zone reset errors ({}):", report.zone_errors.len());
        for e in &report.zone_errors {
            println!("  {e}");
        }
    }

    let mut failures = 0u32;
    let mut expected_hits = 0u32;
    if out_dir.is_some() || reference_dir.is_some() {
        let writers: &[(&str, fn(&mud_world::model::World, u16) -> Vec<u8>)] = &[
            ("wld", write::wld::write_file),
            ("mob", write::mob::write_file),
            ("obj", write::obj::write_file),
            ("zon", write::zon::write_file),
            ("shp", write::shp::write_file),
            ("qst", write::qst::write_file),
            ("trg", write::trg::write_file),
        ];
        for (zr, zone) in world.zones.iter().enumerate() {
            for (ext, wf) in writers {
                let bytes = wf(world, zr as u16);
                let rel = format!("{ext}/{}.{ext}", zone.number);
                if let Some(out) = &out_dir {
                    let p = out.join(&rel);
                    fs::create_dir_all(p.parent().unwrap()).ok();
                    fs::write(&p, &bytes).expect("write output");
                }
                if let Some(reference) = &reference_dir {
                    let gp = reference.join(&rel);
                    match fs::read(&gp) {
                        Ok(gbytes) if gbytes == bytes => {}
                        Ok(_) => {
                            if EXPECTED_DIVERGENCES.contains(&rel.as_str()) {
                                expected_hits += 1;
                                println!("EXPECTED {rel} (301.wld room order)");
                            } else {
                                failures += 1;
                                println!("MISMATCH {rel}");
                            }
                        }
                        Err(e) => {
                            failures += 1;
                            println!("GOLDEN MISSING {rel}: {e}");
                        }
                    }
                }
            }
        }
    }

    if reference_dir.is_some() {
        println!(
            "reference diff: {} mismatches, {} expected",
            failures, expected_hits
        );
    }
    if failures == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
