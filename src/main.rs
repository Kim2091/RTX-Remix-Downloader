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
        "https://raw.githubusercontent.com/NVIDIAGameWorks/bridge-remix/refs/heads/main/bridge.conf",
        ".trex",
    ),
];

// License files to download
const LICENSES: [(&str, &str); 3] = [
    ("LICENSE.txt", "https://raw.githubusercontent.com/NVIDIAGameWorks/rtx-remix/refs/heads/main/LICENSE.txt"),
    ("ThirdPartyLicenses-dxvk.txt", "https://raw.githubusercontent.com/NVIDIAGameWorks/dxvk-remix/refs/heads/main/ThirdPartyLicenses.txt"),
    ("ThirdPartyLicenses-bridge.txt", "https://raw.githubusercontent.com/NVIDIAGameWorks/bridge-remix/refs/heads/main/ThirdPartyLicenses.txt"),
];

fn main() {
    // Run the main logic and handle any errors
    if let Err(e) = run_main() {
        eprintln!("{}", format!("Error: {}", e).red());
        // Keep console open on error
        println!("\nPress Enter to exit...");
        let mut input = String::new();
        let _ = io::stdin().read_line(&mut input);
        std::process::exit(1);
    }
}

fn run_main() -> Result<()> {
    println!("{}", "RTX Remix Download Script v0.4.0".green().bold());

    // First ask about stable vs development
    println!("\nChoose build stream:");
    println!(
        "{}. Stable Release (Use these for the most stable experience)",
        "1".yellow()
    );
    println!(
        "{}. Development Build (Use this for the latest features, but it may be unstable)",
        "2".yellow()
    );

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let is_stable = match input.trim() {
        "1" => true,
        "2" => false,
        _ => {
            println!("Invalid selection, defaulting to stable release");
            true
        }
    };

    // Ask about game architecture type
    println!("\nChoose game type:");
    println!("{}. 32-bit (x86) Games (Most older games)", "1".yellow());
    println!("{}. 64-bit (x64) Games (More modern games)", "2".yellow());

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let is_x86 = match input.trim() {
        "1" => true,
        "2" => false,
        _ => {
            println!("Invalid selection, defaulting to x86");
            true
        }
    };

    // Ask for build type
    println!("\nChoose a build type (type the number and press Enter):");
    for (i, build_type) in BUILD_TYPES.iter().enumerate() {
        println!("{}. {}", (i + 1).to_string().yellow(), build_type);
    }

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let build_type = BUILD_TYPES[input.trim().parse::<usize>()? - 1];

    let client = Client::builder()
        .user_agent("RTX Remix Downloader")
        .build()?;

    // Create and clean the "remix" folder in the current working directory
    let remix_path = PathBuf::from("remix");
    cleanup_existing_directory(&remix_path)?;
    let final_path = remix_path.canonicalize()?;
    if is_stable {
        println!(
            "{}",
            format!("\nDownloading stable {} build...", build_type).cyan()
        );

        // Fetch and download stable release
        let (asset_name, download_url) = fetch_latest_stable_release(&client, build_type)?;
        let stable_zip = final_path.join("stable-release.zip");

        println!("Downloading stable release from GitHub...");
        download_file(&client, &download_url, &stable_zip)?;

        println!("Extracting stable release...");
        let file = fs::File::open(&stable_zip)?;
        let mut archive = zip::ZipArchive::new(file)?;
        archive.extract(&final_path)?;

        // Cleanup zip file
        fs::remove_file(stable_zip)?;

        // Clean up debug files
        cleanup_debug_files(&final_path)?;

        // Write build info with actual package name
        write_build_names(&final_path, &[asset_name])?;

        if is_x86 {
            // Remove d3d8to9.dll and its license file for stable x86 builds
            let d3d8to9_path = final_path.join("d3d8to9.dll");
            let d3d8to9_license_path = final_path.join("ThirdPartyLicenses-d3d8to9.txt");

            if d3d8to9_path.exists() {
                if let Err(e) = fs::remove_file(&d3d8to9_path) {
                    eprintln!(
                        "{}",
                        format!("Warning: Could not remove d3d8to9.dll: {}", e).yellow()
                    );
                } else {
                    println!("{}", "Removed d3d8to9.dll".cyan());
                }
            }

            if d3d8to9_license_path.exists() {
                if let Err(e) = fs::remove_file(&d3d8to9_license_path) {
                    eprintln!(
                        "{}",
                        format!(
                            "Warning: Could not remove ThirdPartyLicenses-d3d8to9.txt: {}",
                            e
                        )
                        .yellow()
                    );
                } else {
                    println!("{}", "Removed ThirdPartyLicenses-d3d8to9.txt".cyan());
                }
            }

            // Download and extract dx8 binaries for x86
            download_and_extract_dx8_binaries(&client, &final_path)?;
            // Download all additional files and licenses
            download_additional_files(&client, &final_path)?;
            download_licenses(&client, &final_path)?;
        } else {
            // For x64, reorganize files and only keep DXVK-related files
            reorganize_x64_files(&final_path)?;
            // Download only DXVK-related licenses
            download_x64_licenses(&client, &final_path)?;
        }
    } else if is_x86 {
        // Fetch and download unified x86 package
        let (artifact_name, download_url) = fetch_x86_unified_artifact(&client, build_type)?;
        let unified_zip = final_path.join("rtx-remix-x86.zip");

        println!("Downloading unified x86 package: {}", artifact_name);
        download_file(&client, &download_url, &unified_zip)?;

        println!("Extracting unified package...");
        let file = fs::File::open(&unified_zip)?;
        let mut archive = zip::ZipArchive::new(file)?;
        archive.extract(&final_path)?;

        // Cleanup zip file
        fs::remove_file(unified_zip)?;

        // Clean up debug files
        cleanup_debug_files(&final_path)?;

        // Download and extract dx8 binaries for x86
        download_and_extract_dx8_binaries(&client, &final_path)?;

        // Download additional files and licenses
        download_additional_files(&client, &final_path)?;
        download_licenses(&client, &final_path)?;

        // Write build info
        write_build_names(&final_path, &[artifact_name])?;
    } else {
        // Fetch and download x64 package
        let (artifact_name, download_url) = fetch_x64_artifact(&client, build_type)?;
        let x64_zip = final_path.join("rtx-remix-x64.zip");

        println!("Downloading x64 package: {}", artifact_name);
        download_file(&client, &download_url, &x64_zip)?;

        println!("Extracting x64 package...");
        let file = fs::File::open(&x64_zip)?;
        let mut archive = zip::ZipArchive::new(file)?;
        archive.extract(&final_path)?;

        // Cleanup zip file
        fs::remove_file(x64_zip)?;

        // Clean up debug files
        cleanup_debug_files(&final_path)?;

        // For x64, only download DXVK-related licenses
        download_x64_licenses(&client, &final_path)?;

        // Write build info
        write_build_names(&final_path, &[artifact_name])?;
    }

    println!("{}", "Download complete!".green().bold());
    println!("You can find the latest RTX Remix install in:");
    println!("{}", clickable_path(&final_path));
    println!("{}", "RTX Remix install guide:".yellow());
    println!(
        "{}",
        "https://github.com/NVIDIAGameWorks/rtx-remix/wiki/runtime-user-guide".cyan()
    );

    // Keep the console open
    println!("\nPress Enter to exit...");
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    Ok(())
}

// === GitHub API Interaction Functions ===
fn fetch_latest_stable_release(client: &Client, build_type: &str) -> Result<(String, String)> {
    println!("{}", "Fetching latest stable release information...".cyan());

    let releases_url = "https://api.github.com/repos/NVIDIAGameWorks/rtx-remix/releases/latest";
    let response: Value = client.get(releases_url).send()?.json()?;

    let asset = response["assets"]
        .as_array()
        .and_then(|assets| {
            assets.iter().find(|asset| {
                asset["name"].as_str().is_some_and(|name| {
                    // Match the exact pattern: ends with build_type.zip
                    // and explicitly exclude -symbols
                    name.ends_with(&format!("-{}.zip", build_type)) && !name.contains("-symbols")
                })
            })
        })
        .context("No suitable release package found")?;

    let download_url = asset["browser_download_url"]
        .as_str()
        .context("No download URL found")?
        .to_string();

    let asset_name = asset["name"]
        .as_str()
        .context("No asset name found")?
        .to_string();

    println!(
        "{}",
        format!("Found stable release: {} ({})", asset_name, download_url).green()
    );

    Ok((asset_name, download_url))
}

fn fetch_x86_unified_artifact(client: &Client, build_type: &str) -> Result<(String, String)> {
    println!(
        "{}",
        format!("Fetching unified x86 package ({} build)...", build_type).cyan()
    );

    let runs_url = format!(
        "https://api.github.com/repos/{}/actions/runs",
        DXVK_REMIX_REPO
    );
    let runs: Value = client.get(runs_url).send()?.json()?;

    let artifacts_url = runs["workflow_runs"]
        .as_array()
        .and_then(|runs| runs.iter().find(|run| run["conclusion"] == "success"))
        .and_then(|run| run["artifacts_url"].as_str())
        .context("No successful run found")?;

    let artifacts: Value = client.get(artifacts_url).send()?.json()?;

    let artifact = artifacts["artifacts"]
        .as_array()
        .and_then(|artifacts| {
            artifacts.iter().find(|a| {
                a["name"].as_str().is_some_and(|name| {
                    name.contains(build_type) && name.contains("rtx-remix-for-x86-games")
                })
            })
        })
        .context("No matching x86 unified artifact found")?;

    let artifact_name = artifact["name"].as_str().unwrap().to_string();
    let artifact_id = artifact["id"].as_u64().unwrap();

    let download_url = format!(
        "https://nightly.link/{}/actions/artifacts/{}.zip",
        DXVK_REMIX_REPO, artifact_id
    );

    Ok((artifact_name, download_url))
}

fn fetch_x64_artifact(client: &Client, build_type: &str) -> Result<(String, String)> {
    println!(
        "{}",
        format!("Fetching x64 package ({} build)...", build_type).cyan()
    );

    let runs_url = format!(
        "https://api.github.com/repos/{}/actions/runs",
        DXVK_REMIX_REPO
    );
    let runs: Value = client.get(runs_url).send()?.json()?;

    let artifacts_url = runs["workflow_runs"]
        .as_array()
        .and_then(|runs| runs.iter().find(|run| run["conclusion"] == "success"))
        .and_then(|run| run["artifacts_url"].as_str())
        .context("No successful run found")?;

    let artifacts: Value = client.get(artifacts_url).send()?.json()?;

    let artifact = artifacts["artifacts"]
        .as_array()
        .and_then(|artifacts| {
            artifacts.iter().find(|a| {
                a["name"].as_str().is_some_and(|name| {
                    name.contains(build_type) && !name.contains("x86") && !name.contains("symbols")
                })
            })
        })
        .context("No matching x64 artifact found")?;

    let artifact_name = artifact["name"].as_str().unwrap().to_string();
    let artifact_id = artifact["id"].as_u64().unwrap();

    let download_url = format!(
        "https://nightly.link/{}/actions/artifacts/{}.zip",
        DXVK_REMIX_REPO, artifact_id
    );

    Ok((artifact_name, download_url))
}

// === Download and File Operations ===
fn download_file(client: &Client, url: &str, dest: &Path) -> Result<()> {
    let mut response = client.get(url).send()?;
    let total_size = response.content_length().unwrap_or(0);

    let pb = ProgressBar::new(total_size);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
        .unwrap()
        .progress_chars("#>-"));

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

fn download_additional_files(client: &Client, final_path: &Path) -> Result<()> {
    println!("{}", "Downloading additional files".cyan());
    for (name, url, destination) in ADDITIONAL_FILES {
        let dest_path = final_path.join(destination).join(name);
        download_file(client, url, &dest_path)?;
        println!("{}", format!("Downloaded {}", name).green());
    }
    Ok(())
}

fn download_licenses(client: &Client, final_path: &Path) -> Result<()> {
    println!("{}", "Downloading license files".cyan());
    for (filename, url) in LICENSES {
        let dest_path = final_path.join(filename);
        download_file(client, url, &dest_path)?;
    }
    println!("{}", "License files downloaded".green());
    Ok(())
}

fn download_x64_licenses(client: &Client, final_path: &Path) -> Result<()> {
    println!("{}", "Downloading license files".cyan());
    // Only download main license and DXVK license for x64 builds
    let x64_licenses = [
        ("LICENSE.txt", "https://raw.githubusercontent.com/NVIDIAGameWorks/rtx-remix/refs/heads/main/LICENSE.txt"),
        ("ThirdPartyLicenses.txt", "https://raw.githubusercontent.com/NVIDIAGameWorks/dxvk-remix/refs/heads/main/ThirdPartyLicenses.txt"),
    ];

    for (filename, url) in x64_licenses {
        let dest_path = final_path.join(filename);
        download_file(client, url, &dest_path)?;
    }
    println!("{}", "License files downloaded".green());
    Ok(())
}

fn download_and_extract_dx8_binaries(client: &Client, final_path: &Path) -> Result<()> {
    println!("{}", "Downloading dx8 binaries".cyan());
    let dx8_url =
        "https://nightly.link/elishacloud/dxwrapper/workflows/ci/master/dx8%20game%20binaries.zip";
    let dx8_zip_path = final_path.join("dx8_binaries.zip");
    download_file(client, dx8_url, &dx8_zip_path)?;

    println!("{}", "Extracting dx8 binaries".cyan());
    let file = fs::File::open(&dx8_zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    archive.extract(final_path)?;

    // Rename d3d8.dll to d3d8_off.dll
    let d3d8_path = final_path.join("d3d8.dll");
    if d3d8_path.exists() {
        fs::rename(&d3d8_path, final_path.join("d3d8_off.dll"))?;
    }

    // Remove d3d8to9.dll
    let d3d8to9_path = final_path.join("d3d8to9.dll");
    if d3d8to9_path.exists() {
        if let Err(e) = fs::remove_file(&d3d8to9_path) {
            eprintln!(
                "{}",
                format!("Warning: Could not remove d3d8to9.dll: {}", e).yellow()
            );
        } else {
            println!("{}", "Removed d3d8to9.dll".cyan());
        }
    }

    // Download the dxwrapper license specifically here since it's related to these binaries
    println!("{}", "Downloading dxwrapper license".cyan());
    let dxwrapper_license_url =
        "https://raw.githubusercontent.com/elishacloud/dxwrapper/refs/heads/master/License.txt";
    let license_dest_path = final_path.join("ThirdPartyLicenses-dxwrapper.txt");
    download_file(client, dxwrapper_license_url, &license_dest_path)?;

    println!("{}", "Cleaning up dx8 binaries zip file".cyan());
    fs::remove_file(dx8_zip_path)?;

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
            format!(
                "Cleaned up {} debugging symbol files and unnecessary files",
                removed_files
            )
            .cyan()
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
        } else {
            let file_name = path.file_name().unwrap_or_default().to_string_lossy();

            if path.extension().is_some_and(|ext| ext == "pdb")
                || file_name == "CRC.txt"
                || file_name == "artifacts_readme.txt"
            {
                if let Err(e) = fs::remove_file(&path) {
                    eprintln!("Failed to remove file {}: {}", path.display(), e);
                } else {
                    *removed_files += 1;
                }
            }
        }
    }
    Ok(())
}

fn reorganize_x64_files(final_path: &Path) -> Result<()> {
    println!("{}", "Reorganizing x64 files...".cyan());

    let trex_path = final_path.join(".trex");
    if !trex_path.exists() {
        // If .trex doesn't exist, maybe it's an older stable release structure?
        // Check if dxvk.dll exists in the root as a fallback check.
        if !final_path.join("dxvk.dll").exists() {
            return Err(anyhow::anyhow!("Could not find .trex directory or dxvk.dll in the package root. Structure might be unexpected."));
        }
        // If dxvk.dll is in root, assume it's already somewhat organized, skip .trex steps.
        println!(
            "{}",
            "Skipping .trex reorganization as dxvk.dll found in root.".yellow()
        );
    } else {
        // Remove nvremixbridge.exe if it exists within .trex
        let bridge_path = trex_path.join("nvremixbridge.exe");
        if bridge_path.exists() {
            fs::remove_file(bridge_path)?;
        }

        // Move all files from .trex to root directory
        for entry in fs::read_dir(&trex_path)? {
            let entry = entry?;
            let source_path = entry.path();
            if source_path.is_file() {
                let file_name = source_path.file_name().unwrap();
                let dest_path = final_path.join(file_name);
                // Overwrite if exists, as we prioritize files from .trex
                if dest_path.exists() {
                    fs::remove_file(&dest_path)?;
                }
                fs::rename(&source_path, &dest_path)?;
            }
        }

        // Move the usd folder from .trex to root if it exists
        let usd_path = trex_path.join("usd");
        let new_usd_path = final_path.join("usd");
        if usd_path.exists() {
            // Remove existing usd folder in root if it exists, to avoid merge conflicts
            if new_usd_path.exists() {
                fs::remove_dir_all(&new_usd_path)?;
            }
            fs::rename(&usd_path, &new_usd_path)?;
        }

        // Now remove the potentially empty .trex directory
        fs::remove_dir_all(&trex_path)?;
    }

    // Remove any bridge, d3d8to9 or d3d8 related files from root (ensure clean state)
    let files_to_remove = [
        "nvremixbridge.exe",
        "d3d8to9.dll",
        "d3d8.dll",
        "d3d8_off.dll",
        "dxwrapper.dll",
        "dxwrapper.ini",
    ];
    for file in files_to_remove.iter() {
        let file_path = final_path.join(file);
        if file_path.exists() {
            fs::remove_file(file_path)?;
        }
    }

    println!("{}", "Files reorganized successfully for x64".green());
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
        format!(
            "Created build-names.txt with {} build names",
            build_names.len()
        )
        .green()
    );
    Ok(())
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace(r"\\?\", "")
}

fn clickable_path(path: &Path) -> String {
    let clean_path = display_path(path);
    format!(
        "\x1B]8;;file://{}\x07{}\x1B]8;;\x07",
        clean_path, clean_path
    )
    .cyan()
    .to_string()
}
