use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use colored::*;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::blocking::Client;
use serde_json::Value;

const BUILD_TYPES: [&str; 3] = ["release", "debugoptimized", "debug"];
const REPOSITORIES: [(&str, &str); 2] = [
    ("NVIDIAGameWorks/dxvk-remix", ".trex"),
    ("NVIDIAGameWorks/bridge-remix", ""),
];
const ADDITIONAL_FILES: [(&str, &str, &str); 1] = [(
    "dxvk.conf",
    "https://raw.githubusercontent.com/NVIDIAGameWorks/dxvk-remix/main/dxvk.conf",
    "",
)];
const LICENSES: [(&str, &str); 4] = [
    ("LICENSE.txt", "https://raw.githubusercontent.com/NVIDIAGameWorks/rtx-remix/refs/heads/main/LICENSE.txt"),
    ("ThirdPartyLicenses-dxvk.txt", "https://raw.githubusercontent.com/NVIDIAGameWorks/dxvk-remix/refs/heads/main/ThirdPartyLicenses.txt"),
    ("ThirdPartyLicenses-bridge.txt", "https://raw.githubusercontent.com/NVIDIAGameWorks/bridge-remix/refs/heads/main/ThirdPartyLicenses.txt"),
    ("ThirdPartyLicenses-dxwrapper.txt", "https://raw.githubusercontent.com/elishacloud/dxwrapper/refs/heads/master/License.txt"),
];

fn main() -> Result<()> {
    println!("{}", "RTX Remix Download Script".green().bold());
    println!("Choose a build type (type the number and press Enter):");
    for (i, build_type) in BUILD_TYPES.iter().enumerate() {
        println!("{}. {}", (i + 1).to_string().yellow(), build_type);
    }

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let build_type = BUILD_TYPES[input.trim().parse::<usize>()? - 1];

    println!("{}", format!("Downloading {} builds", build_type).cyan());

    let client = Client::builder()
        .user_agent("RTX Remix Downloader")
        .build()?;

    // Create the "remix" folder in the current working directory
    let remix_path = PathBuf::from("remix");
    fs::create_dir_all(&remix_path)?;

    // Get the canonicalized path
    let final_path = remix_path.canonicalize()?;

    println!(
        "{}",
        format!("Created remix folder: {}", clickable_path(&final_path)).green()
    );

    for &(repo, subfolder) in &REPOSITORIES {
        match fetch_artifact(&client, repo, build_type) {
            Ok(artifact) => {
                if let Err(e) =
                    download_and_extract_artifact(&client, repo, &artifact, &final_path, subfolder)
                {
                    eprintln!(
                        "{}",
                        format!("Error downloading/extracting artifact: {}", e).red()
                    );
                }
            }
            Err(e) => eprintln!(
                "{}",
                format!("Error fetching artifact for {}: {}", repo, e).red()
            ),
        }
    }

    download_additional_files(&client, &final_path)?;
    download_licenses(&client, &final_path)?;
    cleanup_debug_files(&final_path)?;
    download_and_extract_dx8_binaries(&client, &final_path)?;

    println!("{}", "Download complete!".green().bold());
    println!("You can find the latest RTX Remix install in:");
    println!("{}", clickable_path(&final_path));
    println!("{}", "RTX Remix install guide:".yellow());
    println!(
        "{}",
        "https://github.com/NVIDIAGameWorks/rtx-remix/wiki/runtime-user-guide".cyan()
    );

    // Add this new section to keep the console open
    println!("\nPress Enter to exit...");
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    Ok(())
}

fn fetch_artifact(client: &Client, repo: &str, build_type: &str) -> Result<(String, u64)> {
    println!(
        "{}",
        format!("Fetching artifact for {} ({})", repo, build_type).cyan()
    );

    let runs_url = format!("https://api.github.com/repos/{}/actions/runs", repo);
    let runs: Value = client.get(&runs_url).send()?.json()?;

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
                a["name"]
                    .as_str()
                    .map_or(false, |name| name.contains(build_type))
            })
        })
        .context("No matching artifact found")?;

    let artifact_name = artifact["name"].as_str().unwrap().to_string();
    let artifact_id = artifact["id"].as_u64().unwrap();

    println!(
        "{}",
        format!("Found artifact: {} (ID: {})", artifact_name, artifact_id).green()
    );

    Ok((artifact_name, artifact_id))
}

fn download_and_extract_artifact(
    client: &Client,
    repo: &str,
    artifact: &(String, u64),
    final_path: &Path,
    subfolder: &str,
) -> Result<()> {
    let (artifact_name, artifact_id) = artifact;
    let download_url = format!(
        "https://nightly.link/{}/actions/artifacts/{}.zip",
        repo, artifact_id
    );

    println!(
        "{}",
        format!("Downloading artifact: {}", artifact_name).cyan()
    );
    let mut response = client.get(&download_url).send()?;
    let total_size = response.content_length().unwrap_or(0);

    let pb = ProgressBar::new(total_size);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
        .unwrap()
        .progress_chars("#>-"));

    let path = final_path
        .join(subfolder)
        .join(format!("{}.zip", artifact_name));
    fs::create_dir_all(path.parent().unwrap())?;
    let mut file = fs::File::create(&path)?;

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

    println!(
        "{}",
        format!("Extracting artifact: {}", artifact_name).cyan()
    );
    let file = fs::File::open(&path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    archive.extract(path.parent().unwrap())?;

    fs::remove_file(path)?;

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

            if path.extension().map_or(false, |ext| ext == "pdb")
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

fn download_and_extract_dx8_binaries(client: &Client, final_path: &Path) -> Result<()> {
    println!("{}", "Downloading dx8 binaries".cyan());
    let dx8_url =
        "https://nightly.link/elishacloud/dxwrapper/workflows/ci/master/dx8%20binaries.zip";
    let dx8_zip_path = final_path.join("dx8_binaries.zip");
    download_file(client, dx8_url, &dx8_zip_path)?;

    println!("{}", "Extracting dx8 binaries".cyan());
    let file = fs::File::open(&dx8_zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    archive.extract(final_path)?;

    let d3d8_path = final_path.join("d3d8.dll");
    if d3d8_path.exists() {
        fs::rename(&d3d8_path, final_path.join("d3d8_off.dll"))?;
    }

    println!("{}", "Cleaning up dx8 binaries zip file".cyan());
    fs::remove_file(dx8_zip_path)?;

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
