use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use colored::*;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::blocking::Client;
use serde_json::Value;

// === Constants ===
const BUILD_TYPES: [&str; 3] = ["release", "debugoptimized", "debug"];
const DXVK_REMIX_REPO: &str = "NVIDIAGameWorks/dxvk-remix";

// Configuration files to download
const ADDITIONAL_FILES: [(&str, &str, &str); 2] = [
    (
        "dxvk.conf",
        "https://raw.githubusercontent.com/NVIDIAGameWorks/dxvk-remix/main/dxvk.conf",
        "",
    ),
    (
        "bridge.conf",
        "https://raw.githubusercontent.com/NVIDIAGameWorks/dxvk-remix/refs/heads/main/bridge/bridge.conf",
        ".trex",
    ),
];

// License files to download
const LICENSES: [(&str, &str); 3] = [
    ("LICENSE.txt", "https://raw.githubusercontent.com/NVIDIAGameWorks/rtx-remix/refs/heads/main/LICENSE.txt"),
    ("ThirdPartyLicenses-dxvk.txt", "https://raw.githubusercontent.com/NVIDIAGameWorks/dxvk-remix/refs/heads/main/ThirdPartyLicenses.txt"),
    ("ThirdPartyLicenses-bridge.txt", "https://raw.githubusercontent.com/NVIDIAGameWorks/dxvk-remix/refs/heads/main/bridge/ThirdPartyLicenses.txt"),
];

const X64_LICENSES: [(&str, &str); 2] = [
    ("LICENSE.txt", "https://raw.githubusercontent.com/NVIDIAGameWorks/rtx-remix/refs/heads/main/LICENSE.txt"),
    ("ThirdPartyLicenses.txt", "https://raw.githubusercontent.com/NVIDIAGameWorks/dxvk-remix/refs/heads/main/ThirdPartyLicenses.txt"),
];

// === Configuration Structures ===
#[derive(Debug, Clone, Copy)]
struct BuildConfig {
    is_stable: bool,
    is_x86: bool,
    build_type: &'static str,
}

// === Main Entry Point ===
fn main() {
    if let Err(e) = run_main() {
        eprintln!("{}", format!("Error: {}", e).red());
        println!("\nPress Enter to exit...");
        let mut input = String::new();
        let _ = io::stdin().read_line(&mut input);
        std::process::exit(1);
    }
}

fn run_main() -> Result<()> {
    println!("{}", "RTX Remix Download Script v0.3.0.2".green().bold());

    let config = prompt_user_configuration()?;
    let client = create_http_client()?;
    let remix_path = setup_output_directory()?;

    download_and_install(&client, &remix_path, config)?;
    display_completion_message(&remix_path);

    wait_for_exit();
    Ok(())
}

// === User Input Functions ===
fn prompt_user_configuration() -> Result<BuildConfig> {
    let is_stable = prompt_build_stream()?;
    let is_x86 = prompt_architecture()?;
    let build_type = prompt_build_type()?;

    Ok(BuildConfig {
        is_stable,
        is_x86,
        build_type,
    })
}

fn prompt_build_stream() -> Result<bool> {
    prompt_choice(
        "\nChoose build stream:",
        &[
            "Stable Release (Use these for the most stable experience)",
            "Development Build (Use this for the latest features, but it may be unstable)",
        ],
        true,
        "stable release"
    )
}

fn prompt_architecture() -> Result<bool> {
    prompt_choice(
        "\nChoose game type:",
        &[
            "32-bit (x86) Games (Most older games)",
            "64-bit (x64) Games (More modern games)",
        ],
        true,
        "x86"
    )
}

fn prompt_choice(title: &str, options: &[&str], default_first: bool, default_name: &str) -> Result<bool> {
    println!("{}", title);
    for (i, option) in options.iter().enumerate() {
        println!("{}. {}", (i + 1).to_string().yellow(), option);
    }

    let input = read_user_input()?;
    match input.trim() {
        "1" => Ok(true),
        "2" => Ok(false),
        _ => {
            println!("Invalid selection, defaulting to {}", default_name);
            Ok(default_first)
        }
    }
}

fn prompt_build_type() -> Result<&'static str> {
    println!("\nChoose a build type (type the number and press Enter):");
    for (i, build_type) in BUILD_TYPES.iter().enumerate() {
        println!("{}. {}", (i + 1).to_string().yellow(), build_type);
    }

    let input = read_user_input()?;
    let index = input.trim().parse::<usize>()
        .context("Invalid build type selection")?
        .checked_sub(1)
        .context("Invalid build type number")?;
    
    BUILD_TYPES.get(index)
        .copied()
        .context("Build type index out of range")
}

fn read_user_input() -> Result<String> {
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input)
}

// === Setup Functions ===
fn create_http_client() -> Result<Client> {
    Client::builder()
        .user_agent("RTX Remix Downloader")
        .build()
        .context("Failed to create HTTP client")
}

fn setup_output_directory() -> Result<PathBuf> {
    let remix_path = PathBuf::from("remix");
    cleanup_existing_directory(&remix_path)?;
    remix_path.canonicalize().context("Failed to canonicalize remix path")
}

// === Main Download and Installation Logic ===
fn download_and_install(client: &Client, final_path: &Path, config: BuildConfig) -> Result<()> {
    println!("{}", format!("\nDownloading {} {} build...", 
        if config.is_stable { "stable" } else { "development" },
        config.build_type
    ).cyan());

    let (artifact_name, download_url) = fetch_package(client, config)?;
    let zip_path = final_path.join("package.zip");

    download_file(client, &download_url, &zip_path)?;
    extract_archive(&zip_path, final_path)?;
    fs::remove_file(zip_path)?;
    
    // Only cleanup debug files for stable builds
    if config.is_stable {
        cleanup_debug_files(final_path)?;
    }

    if config.is_stable && config.is_x86 {
        handle_stable_x86_cleanup(final_path)?;
    }

    if config.is_x86 {
        download_and_extract_dx8_binaries(client, final_path)?;
        download_additional_files(client, final_path)?;
        download_licenses(client, final_path)?;
    } else if config.is_stable {
        // Only reorganize and download licenses for stable x64 builds
        reorganize_x64_files(final_path)?;
        download_dxvk_conf_only(client, final_path)?;
        download_x64_licenses(client, final_path)?;
    } else {
        // For development x64 builds, just add dxvk.conf to root
        download_dxvk_conf_only(client, final_path)?;
    }
    write_build_names(final_path, &[artifact_name])?;
    Ok(())
}

fn fetch_package(client: &Client, config: BuildConfig) -> Result<(String, String)> {
    if config.is_stable {
        fetch_latest_stable_release(client, config.build_type)
    } else if config.is_x86 {
        fetch_x86_unified_artifact(client, config.build_type)
    } else {
        fetch_x64_artifact(client, config.build_type)
    }
}

// === GitHub API Interaction Functions ===
fn fetch_latest_stable_release(client: &Client, build_type: &str) -> Result<(String, String)> {
    println!("{}", "Fetching latest stable release information...".cyan());

    let releases_url = "https://api.github.com/repos/NVIDIAGameWorks/rtx-remix/releases/latest";
    let response: Value = client.get(releases_url).send()?.json()?;

    let asset = find_release_asset(&response, build_type)?;
    let download_url = extract_asset_url(asset)?;
    let asset_name = extract_asset_name(asset)?;

    println!("{}", format!("Found stable release: {} ({})", asset_name, download_url).green());
    Ok((asset_name, download_url))
}

fn find_release_asset<'a>(response: &'a Value, build_type: &str) -> Result<&'a Value> {
    response["assets"]
        .as_array()
        .and_then(|assets| {
            assets.iter().find(|asset| {
                asset["name"].as_str().is_some_and(|name| {
                    name.ends_with(&format!("-{}.zip", build_type)) && !name.contains("-symbols")
                })
            })
        })
        .context("No suitable release package found")
}

fn fetch_x86_unified_artifact(client: &Client, build_type: &str) -> Result<(String, String)> {
    println!("{}", format!("Fetching unified x86 package ({} build)...", build_type).cyan());
    fetch_artifact(client, build_type, |name| {
        name.contains(build_type) && name.contains("rtx-remix-for-x86-games")
    })
}

fn fetch_x64_artifact(client: &Client, build_type: &str) -> Result<(String, String)> {
    println!("{}", format!("Fetching x64 package ({} build)...", build_type).cyan());
    fetch_artifact(client, build_type, |name| {
        name.contains(build_type) && !name.contains("x86") && !name.contains("symbols")
    })
}

fn fetch_artifact<F>(client: &Client, _build_type: &str, filter: F) -> Result<(String, String)>
where
    F: Fn(&str) -> bool,
{
        let runs_url = format!(
        "https://api.github.com/repos/{}/actions/runs?branch=main",
        DXVK_REMIX_REPO
    );
    let runs: Value = client.get(runs_url).send()?.json()?;

    let latest_successful_run = find_latest_successful_run(&runs)?;
    let artifacts_url = extract_artifacts_url(latest_successful_run)?;
    let artifacts: Value = client.get(artifacts_url).send()?.json()?;

    let artifact = find_matching_artifact(&artifacts, filter)?;
    let artifact_name = extract_artifact_name(artifact)?;
    let artifact_id = extract_artifact_id(artifact)?;

    let download_url = format!(
        "https://nightly.link/{}/actions/artifacts/{}.zip",
        DXVK_REMIX_REPO, artifact_id
    );

    Ok((artifact_name, download_url))
}

fn find_latest_successful_run(runs: &Value) -> Result<&Value> {
    runs["workflow_runs"]
        .as_array()
        .and_then(|runs_array| {
            runs_array
                .iter()
                .filter(|run| run["conclusion"] == "success")
                .max_by_key(|run| run["run_number"].as_u64().unwrap_or(0))
        })
        .context("No successful run found on the main branch")
}

fn find_matching_artifact<F>(artifacts: &Value, filter: F) -> Result<&Value>
where
    F: Fn(&str) -> bool,
{
    artifacts["artifacts"]
        .as_array()
        .and_then(|artifacts_array| {
            artifacts_array.iter().find(|a| {
                a["name"].as_str().is_some_and(&filter)
            })
        })
        .context("No matching artifact found")
}

// === JSON Extraction Helpers ===
fn extract_json_string(value: &Value, key: &str) -> Result<String> {
    value[key]
        .as_str()
        .map(String::from)
        .context(format!("No {} found", key))
}

fn extract_json_str<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
        value[key]
        .as_str()
        .context(format!("No {} found", key))
}

fn extract_json_u64(value: &Value, key: &str) -> Result<u64> {
    value[key]
        .as_u64()
        .context(format!("No {} found", key))
}

fn extract_asset_url(asset: &Value) -> Result<String> {
    extract_json_string(asset, "browser_download_url")
}

fn extract_asset_name(asset: &Value) -> Result<String> {
    extract_json_string(asset, "name")
}

fn extract_artifacts_url(run: &Value) -> Result<&str> {
    extract_json_str(run, "artifacts_url")
}

fn extract_artifact_name(artifact: &Value) -> Result<String> {
    extract_json_string(artifact, "name")
}

fn extract_artifact_id(artifact: &Value) -> Result<u64> {
    extract_json_u64(artifact, "id")
}

// === Download and File Operations ===
fn download_file(client: &Client, url: &str, dest: &Path) -> Result<()> {
    let mut response = client.get(url).send()?;
    let total_size = response.content_length().unwrap_or(0);

    let pb = create_progress_bar(total_size);
    let mut file = fs::File::create(dest)?;
    let mut buffer = [0; 8192];

    while let Ok(size) = response.read(&mut buffer) {
        if size == 0 {
            break;
        }
        file.write_all(&buffer[..size])?;
        pb.inc(size as u64);
        pb.set_message("Downloading...");
    }

    pb.finish_with_message("Download complete");
    Ok(())
}

fn create_progress_bar(total_size: u64) -> ProgressBar {
    let pb = ProgressBar::new(total_size);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .unwrap()
            .progress_chars("#>-")
    );
    pb
}

fn extract_archive(zip_path: &Path, dest: &Path) -> Result<()> {
    println!("Extracting package...");
    let file = fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    archive.extract(dest)?;
    Ok(())
}

fn download_additional_files(client: &Client, final_path: &Path) -> Result<()> {
    println!("{}", "Downloading additional files".cyan());
    for (name, url, destination) in ADDITIONAL_FILES {
        let dest_path = final_path.join(destination).join(name);
        download_file(client, url, &dest_path)?;
        println!("{}", format!("Downloaded {}", name).green());
    }
    Ok(())
}

fn download_files_to_path(client: &Client, final_path: &Path, files: &[(&str, &str)], message: &str) -> Result<()> {
    println!("{}", message.cyan());
    for (filename, url) in files {
        let dest_path = final_path.join(filename);
        download_file(client, url, &dest_path)?;
    }
    println!("{}", format!("{} complete", message).green());
    Ok(())
}

fn download_licenses(client: &Client, final_path: &Path) -> Result<()> {
    download_files_to_path(client, final_path, &LICENSES, "Downloading license files")
}

fn download_x64_licenses(client: &Client, final_path: &Path) -> Result<()> {
    download_files_to_path(client, final_path, &X64_LICENSES, "Downloading license files")
}

fn download_dxvk_conf_only(client: &Client, final_path: &Path) -> Result<()> {
    let dxvk_conf = [("dxvk.conf", "https://raw.githubusercontent.com/NVIDIAGameWorks/dxvk-remix/main/dxvk.conf")];
    download_files_to_path(client, final_path, &dxvk_conf, "Downloading dxvk.conf")
}

fn download_and_extract_dx8_binaries(client: &Client, final_path: &Path) -> Result<()> {
    println!("{}", "Downloading dx8 binaries".cyan());
    let dx8_url = "https://nightly.link/elishacloud/dxwrapper/workflows/ci/master/dx8%20game%20binaries.zip";
    let dx8_zip_path = final_path.join("dx8_binaries.zip");
    
    download_file(client, dx8_url, &dx8_zip_path)?;
    extract_archive(&dx8_zip_path, final_path)?;
    
    rename_d3d8_dll(final_path)?;
    remove_d3d8to9_dll(final_path)?;
    download_dxwrapper_license(client, final_path)?;
    
    fs::remove_file(dx8_zip_path)?;
    Ok(())
}

fn rename_d3d8_dll(final_path: &Path) -> Result<()> {
    let d3d8_path = final_path.join("d3d8.dll");
    if d3d8_path.exists() {
        fs::rename(&d3d8_path, final_path.join("d3d8_off.dll"))?;
    }
    Ok(())
}

fn remove_d3d8to9_dll(final_path: &Path) -> Result<()> {
    let d3d8to9_path = final_path.join("d3d8to9.dll");
    if d3d8to9_path.exists() {
        if let Err(e) = fs::remove_file(&d3d8to9_path) {
            eprintln!("{}", format!("Warning: Could not remove d3d8to9.dll: {}", e).yellow());
        } else {
            println!("{}", "Removed d3d8to9.dll".cyan());
        }
    }
    Ok(())
}

fn download_dxwrapper_license(client: &Client, final_path: &Path) -> Result<()> {
    println!("{}", "Downloading dxwrapper license".cyan());
    let dxwrapper_license_url = "https://raw.githubusercontent.com/elishacloud/dxwrapper/refs/heads/master/License.txt";
    let license_dest_path = final_path.join("ThirdPartyLicenses-dxwrapper.txt");
    download_file(client, dxwrapper_license_url, &license_dest_path)?;
    Ok(())
}

// === File System Operations ===
fn cleanup_existing_directory(path: &Path) -> Result<()> {
    if path.exists() {
        println!("{}", "Cleaning up existing installation...".cyan());
        fs::remove_dir_all(path)?;
    }
    fs::create_dir_all(path)?;
    Ok(())
}

fn cleanup_debug_files(dir: &Path) -> Result<()> {
    let mut removed_files = 0;
    cleanup_debug_files_recursive(dir, &mut removed_files)?;
    if removed_files > 0 {
        println!(
            "{}",
            format!("Cleaned up {} debugging symbol files and unnecessary files", removed_files).cyan()
        );
    }
    Ok(())
}

fn cleanup_debug_files_recursive(dir: &Path, removed_files: &mut u32) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            cleanup_debug_files_recursive(&path, removed_files)?;
        } else if should_remove_file(&path)
            && fs::remove_file(&path).is_ok() {
            *removed_files += 1;
        }
    }
    Ok(())
}

fn should_remove_file(path: &Path) -> bool {
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    path.extension().is_some_and(|ext| ext == "pdb")
        || file_name == "CRC.txt"
        || file_name == "artifacts_readme.txt"
}

fn handle_stable_x86_cleanup(final_path: &Path) -> Result<()> {
    remove_file_if_exists(final_path, "d3d8to9.dll")?;
    remove_file_if_exists(final_path, "ThirdPartyLicenses-d3d8to9.txt")?;
    Ok(())
}

fn remove_file_if_exists(base_path: &Path, filename: &str) -> Result<()> {
    let file_path = base_path.join(filename);
    if file_path.exists() {
        fs::remove_file(&file_path)
            .unwrap_or_else(|e| eprintln!("{}", format!("Warning: Could not remove {}: {}", filename, e).yellow()));
    }
    Ok(())
}

fn reorganize_x64_files(final_path: &Path) -> Result<()> {
    println!("{}", "Reorganizing x64 files...".cyan());

    let trex_path = final_path.join(".trex");
    if !trex_path.exists() {
        if !final_path.join("dxvk.dll").exists() {
            return Err(anyhow::anyhow!(
                "Could not find .trex directory or dxvk.dll in the package root"
            ));
        }
        println!("{}", "Skipping .trex reorganization as dxvk.dll found in root.".yellow());
    } else {
        process_trex_directory(&trex_path, final_path)?;
    }

    remove_x64_unwanted_files(final_path)?;
    println!("{}", "Files reorganized successfully for x64".green());
    Ok(())
}

fn process_trex_directory(trex_path: &Path, final_path: &Path) -> Result<()> {
    // Remove bridge executable
    let bridge_path = trex_path.join("nvremixbridge.exe");
    if bridge_path.exists() {
        fs::remove_file(bridge_path)?;
    }

    // Move files from .trex to root
    move_files_to_root(trex_path, final_path)?;
    
    // Move usd folder
    move_usd_folder(trex_path, final_path)?;
    
    // Remove .trex directory
    fs::remove_dir_all(trex_path)?;
    Ok(())
}

fn move_files_to_root(trex_path: &Path, final_path: &Path) -> Result<()> {
    // Clear all files in the root directory first
    for entry in fs::read_dir(final_path)? {
        let entry = entry?;
        let path = entry.path();
        
        // Skip the .trex directory itself and any other directories
        if path.is_file() {
            fs::remove_file(&path)?;
        }
    }
    
    // Now move files from .trex to root
    for entry in fs::read_dir(trex_path)? {
        let entry = entry?;
        let source_path = entry.path();
        if source_path.is_file() {
            let file_name = source_path.file_name().unwrap();
            let dest_path = final_path.join(file_name);
            fs::rename(&source_path, &dest_path)?;
        }
    }
    Ok(())
}

fn move_usd_folder(trex_path: &Path, final_path: &Path) -> Result<()> {
    let usd_path = trex_path.join("usd");
    let new_usd_path = final_path.join("usd");
    if usd_path.exists() {
        if new_usd_path.exists() {
            fs::remove_dir_all(&new_usd_path)?;
        }
        fs::rename(&usd_path, &new_usd_path)?;
    }
    Ok(())
}

fn remove_x64_unwanted_files(final_path: &Path) -> Result<()> {
    let files_to_remove = [
        "nvremixbridge.exe",
        "d3d8to9.dll"
    ];
    
    for file in files_to_remove {
        remove_file_if_exists(final_path, file)?;
    }
    Ok(())
}

// === Utility Functions ===
fn write_build_names(final_path: &Path, build_names: &[String]) -> Result<()> {
    let build_names_path = final_path.join("build-names.txt");
    let mut file = fs::File::create(build_names_path)?;
    for name in build_names {
        writeln!(file, "{}", name)?;
    }
    println!(
        "{}",
        format!("Created build-names.txt with {} build names", build_names.len()).green()
    );
    Ok(())
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace(r"\\?\", "")
}

fn clickable_path(path: &Path) -> String {
    let clean_path = display_path(path);
    format!("\x1B]8;;file://{}\x07{}\x1B]8;;\x07", clean_path, clean_path)
        .cyan()
        .to_string()
}

fn display_completion_message(final_path: &Path) {
    println!("{}", "Download complete!".green().bold());
    println!("You can find the latest RTX Remix install in:");
    println!("{}", clickable_path(final_path));
    println!("{}", "RTX Remix install guide:".yellow());
    println!("{}", "https://github.com/NVIDIAGameWorks/rtx-remix/wiki/runtime-user-guide".cyan());
}

fn wait_for_exit() {
    println!("\nPress Enter to exit...");
    let mut input = String::new();
    let _ = io::stdin().read_line(&mut input);
}
