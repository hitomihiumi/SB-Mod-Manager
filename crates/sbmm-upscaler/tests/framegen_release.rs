//! Checks an unpacked dlssg_for_sm86 release against the package rules.
//!
//! The release is 80 MB, so it is not fetched by the suite: unpack one and
//! point `SBMM_SM86_PACKAGE` at the folder to run this.

use std::path::PathBuf;

use sbmm_upscaler::framegen::{files_to_deploy, inspect, Proxy};

#[test]
fn a_real_release_passes_inspection() {
    let Some(root) = std::env::var_os("SBMM_SM86_PACKAGE").map(PathBuf::from) else {
        eprintln!("SBMM_SM86_PACKAGE is not set; skipping");
        return;
    };

    let package = inspect(&root).expect("the published release should pass its own manifest");
    assert!(package.verified);
    assert!(package.version.is_some());

    let files = files_to_deploy(&package, Proxy::Version);
    assert!(files.iter().all(|(source, _)| root.join(source).is_file()));
    assert!(files
        .iter()
        .any(|(_, target)| target == &PathBuf::from("version.dll")));
}
