use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const MAKEMSIX_NAME: &str = "makemsix";
const WINGETUTIL_DLL_NAME: &str = "WinGetUtil.dll";
const BUILD_MAKEMSIX_SCRIPT_RELATIVE_PATH: &str = "scripts/build-makemsix.sh";
const BUILD_WINGETUTIL_SCRIPT_RELATIVE_PATH: &str = "scripts/build-wingetutil.ps1";
const MSIX_PACKAGING_ROOT_ENV: &str = "MSIX_PACKAGING_ROOT";
const WINGET_CLI_ROOT_ENV: &str = "WINGET_CLI_ROOT";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={BUILD_MAKEMSIX_SCRIPT_RELATIVE_PATH}");
    println!("cargo:rerun-if-changed={BUILD_WINGETUTIL_SCRIPT_RELATIVE_PATH}");
    println!("cargo:rerun-if-changed=.gitmodules");
    println!("cargo:rerun-if-env-changed={MSIX_PACKAGING_ROOT_ENV}");
    println!("cargo:rerun-if-env-changed={WINGET_CLI_ROOT_ENV}");

    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set"));
    let destination_dir = build_output_dir();
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();

    match target_os.as_str() {
        "windows" => provision_wingetutil(&manifest_dir, &destination_dir, &target_arch),
        "linux" | "macos" => provision_makemsix(&manifest_dir, &destination_dir, &target_os),
        _ => {}
    }
}

fn resolve_winget_cli_root(workspace_root: &Path) -> Option<PathBuf> {
    if let Ok(env_override) = env::var(WINGET_CLI_ROOT_ENV) {
        let env_override = PathBuf::from(env_override);
        if env_override
            .join("src")
            .join("AppInstallerCLI.sln")
            .is_file()
        {
            return Some(env_override);
        }
    }

    let submodule = workspace_root.join("winget-cli");
    if submodule.join("src").join("AppInstallerCLI.sln").is_file() {
        return Some(submodule);
    }

    None
}

fn resolve_msix_packaging_root(workspace_root: &Path) -> Option<PathBuf> {
    if let Ok(env_override) = env::var(MSIX_PACKAGING_ROOT_ENV) {
        let env_override = PathBuf::from(env_override);
        if env_override.join("CMakeLists.txt").is_file() && env_override.join("src").is_dir() {
            return Some(env_override);
        }
    }

    let submodule = workspace_root.join("msix-packaging");
    if submodule.join("CMakeLists.txt").is_file() && submodule.join("src").is_dir() {
        return Some(submodule);
    }

    None
}

fn build_output_dir() -> PathBuf {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR must be set"));
    out_dir
        .ancestors()
        .nth(3)
        .expect("OUT_DIR should contain a cargo target profile directory")
        .to_path_buf()
}

fn render_output(stdout: &[u8], stderr: &[u8]) -> String {
    let stdout = String::from_utf8_lossy(stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(stderr).trim().to_string();
    let details = [stdout, stderr]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();

    if details.is_empty() {
        "command exited without additional output".to_string()
    } else {
        details.join("\n")
    }
}

fn provision_wingetutil(manifest_dir: &Path, destination_dir: &Path, target_arch: &str) {
    let destination_dll = destination_dir.join(WINGETUTIL_DLL_NAME);
    if let Some(destination_dir) = destination_dll.parent() {
        fs::create_dir_all(destination_dir).expect("failed to create WinGetUtil output directory");
    }

    if destination_dll.is_file() {
        return;
    }

    let configuration = match env::var("PROFILE").as_deref() {
        Ok("release") => "Release",
        _ => "Debug",
    };
    let platform = match target_arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "x86",
        other if !other.is_empty() => other,
        _ => "x64",
    };
    let Some(winget_cli_root) = resolve_winget_cli_root(manifest_dir) else {
        println!(
            "cargo:warning=WinGetUtil.dll was not found at {}. Set {WINGET_CLI_ROOT_ENV} or initialize the bundled winget-cli submodule before running Windows builds.",
            destination_dll.display()
        );
        return;
    };

    let script_path = manifest_dir.join(BUILD_WINGETUTIL_SCRIPT_RELATIVE_PATH);
    if !script_path.is_file() {
        panic!(
            "WinGetUtil build script was not found at {}",
            script_path.display()
        );
    }

    let output = Command::new("powershell")
        .arg("-NoProfile")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-File")
        .arg(&script_path)
        .arg("-WingetCliRoot")
        .arg(&winget_cli_root)
        .arg("-Configuration")
        .arg(configuration)
        .arg("-Platform")
        .arg(platform)
        .arg("-Destination")
        .arg(destination_dir)
        .current_dir(manifest_dir)
        .output()
        .expect("failed to start WinGetUtil build script");

    if !output.status.success() {
        panic!(
            "failed to build WinGetUtil.dll\n{}",
            render_output(&output.stdout, &output.stderr)
        );
    }

    if !destination_dll.is_file() {
        panic!(
            "WinGetUtil.dll was not copied to {}",
            destination_dll.display()
        );
    }
}

fn provision_makemsix(manifest_dir: &Path, destination_dir: &Path, target_os: &str) {
    let makemsix_name = if target_os == "windows" {
        format!("{MAKEMSIX_NAME}.exe")
    } else {
        MAKEMSIX_NAME.to_string()
    };
    let destination_binary = destination_dir.join(&makemsix_name);
    if destination_binary.is_file() {
        return;
    }

    let Some(msix_packaging_root) = resolve_msix_packaging_root(manifest_dir) else {
        println!(
            "cargo:warning=makemsix was not provisioned at {}. Set {MSIX_PACKAGING_ROOT_ENV} or initialize the bundled msix-packaging submodule before running non-Windows packaging tests.",
            destination_binary.display()
        );
        return;
    };

    let script_path = manifest_dir.join(BUILD_MAKEMSIX_SCRIPT_RELATIVE_PATH);
    if !script_path.is_file() {
        panic!(
            "makemsix build script was not found at {}",
            script_path.display()
        );
    }

    let output = Command::new("bash")
        .arg(&script_path)
        .arg("--msix-packaging-root")
        .arg(&msix_packaging_root)
        .arg("--destination")
        .arg(destination_dir)
        .current_dir(manifest_dir)
        .output()
        .expect("failed to start makemsix build script");

    if !output.status.success() {
        panic!(
            "failed to build makemsix\n{}",
            render_output(&output.stdout, &output.stderr)
        );
    }

    if !destination_binary.is_file() {
        panic!(
            "makemsix was not copied to {}",
            destination_binary.display()
        );
    }
}
