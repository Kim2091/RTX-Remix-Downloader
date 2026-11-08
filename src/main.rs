use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use colored::*;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::blocking::Client;
use serde_json::Value;

// === Constants ===
const INSTALL_DIR: &str = "remix";
const STAGING_PREFIX: &str = "remix-download";
const KEPT_FILES_PREFIX: &str = "remix-old";
// Lists the files the downloader installed, so only those are ever deleted.
const MANIFEST_NAME: &str = "downloader-manifest.txt";
const BUILD_TYPES: [&str; 3] = ["release", "debugoptimized", "debug"];

// Folder that holds the game-side files in combined Remix Plus release zips.
const COMBINED_GAME_FOLDER: &str = "game folder";

const DX8_WRAPPER_URL: &str =
    "https://nightly.link/elishacloud/dxwrapper/workflows/ci/master/dx8%20game%20binaries.zip";
// Files taken from the dxwrapper package and their names in the install.
const DX8_WRAPPER_FILES: [(&str, &str); 4] = [
    ("d3d8.dll", "d3d8_off.dll"),
    ("dxwrapper.dll", "dxwrapper.dll"),
    ("dxwrapper.ini", "dxwrapper.ini"),
    ("License.txt", "ThirdPartyLicenses-dxwrapper.txt"),
];

// RTX Remix packages may lack these; they are fetched only when missing.
const REMIX_LICENSES: [(&str, &str); 2] = [
    (
        "LICENSE.txt",
        "https://raw.githubusercontent.com/NVIDIAGameWorks/rtx-remix/main/LICENSE.txt",
    ),
    (
        "ThirdPartyLicenses-dxvk.txt",
        "https://raw.githubusercontent.com/NVIDIAGameWorks/dxvk-remix/main/ThirdPartyLicenses.txt",
    ),
];
const REMIX_BRIDGE_LICENSE: (&str, &str) = (
    "ThirdPartyLicenses-bridge.txt",
    "https://raw.githubusercontent.com/NVIDIAGameWorks/dxvk-remix/main/bridge/ThirdPartyLicenses.txt",
);

// === Configuration Types ===
#[derive(Debug, Clone, Copy, PartialEq)]
enum Product {
    Remix,
    RemixPlus,
}

impl Product {
    fn release_repo(self) -> &'static str {
        match self {
            Product::Remix => "NVIDIAGameWorks/rtx-remix",
            Product::RemixPlus => "RemixProjGroup/dxvk-remix",
        }
    }

    fn runtime_repo(self) -> &'static str {
        match self {
            Product::Remix => "NVIDIAGameWorks/dxvk-remix",
            Product::RemixPlus => "RemixProjGroup/dxvk-remix",
        }
    }

    /// Branch whose CI builds are offered as development builds.
    fn dev_branch(self) -> &'static str {
        match self {
            Product::Remix => "main",
            Product::RemixPlus => "dev",
        }
    }

    fn arches(self) -> &'static [Arch] {
        match self {
            Product::Remix => &[Arch::X86, Arch::X64, Arch::Arm64],
            Product::RemixPlus => &[Arch::X86, Arch::X64],
        }
    }

    /// Raw URL of a file in the runtime repository's main branch.
    fn raw_url(self, path: &str) -> String {
        format!(
            "https://raw.githubusercontent.com/{}/main/{}",
            self.runtime_repo(),
            path
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stream {
    Stable,
    Development,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Arch {
    X86,
    X64,
    Arm64,
}

impl Arch {
    fn slug(self) -> &'static str {
        match self {
            Arch::X86 => "x86",
            Arch::X64 => "x64",
            Arch::Arm64 => "arm64",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Arch::X86 => "32-bit (x86) Games (Most older games)",
            Arch::X64 => "64-bit (x64) Games (More modern games)",
            Arch::Arm64 => "ARM64 Games (Native Windows on ARM games; x86 and x64 games on ARM PCs use the options above)",
        }
    }
}

#[derive(Debug)]
struct Package {
    name: String,
    build_type: &'static str,
    url: String,
}

// === Main Entry Point ===
fn main() {
    if let Err(e) = run() {
        eprintln!("{}", format!("Error: {:#}", e).red());
        wait_for_exit();
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    println!(
        "{}",
        format!("RTX Remix Download Script v{}", env!("CARGO_PKG_VERSION"))
            .green()
            .bold()
    );

    let product = prompt_choice(
        "\nChoose runtime:",
        &[
            ("RTX Remix (Official NVIDIA builds)", Product::Remix),
            (
                "Remix Plus (Community fork by RemixProjGroup, not made or endorsed by NVIDIA)",
                Product::RemixPlus,
            ),
        ],
    )?;
    let stream = prompt_choice(
        "\nChoose build stream:",
        &[
            (
                "Stable Release (Use these for the most stable experience)",
                Stream::Stable,
            ),
            (
                "Development Build (Use this for the latest features, but it may be unstable)",
                Stream::Development,
            ),
        ],
    )?;
    let arch_options: Vec<(&str, Arch)> = product
        .arches()
        .iter()
        .map(|arch| (arch.label(), *arch))
        .collect();
    let arch = prompt_choice("\nChoose game type:", &arch_options)?;

    let client = Client::builder()
        .user_agent("RTX Remix Downloader")
        .build()
        .context("Failed to create HTTP client")?;

    let packages = match stream {
        Stream::Stable => find_stable_packages(&client, product, arch)?,
        Stream::Development => find_dev_packages(&client, product, arch)?,
    };
    let package = choose_build_type(packages)?;

    // The new install is built in its own folder and swapped in only once it is complete.
    let base_dir = install_base_dir()?;
    let staging = create_unique_dir(&base_dir, STAGING_PREFIX)?;
    let built = install(&client, &staging, product, stream, arch, &package)
        .and_then(|()| write_manifest(&staging));
    if let Err(e) = built {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }

    let install_dir = base_dir.join(INSTALL_DIR);
    replace_install(&staging, &install_dir)?;
    display_completion_message(&install_dir, product);

    wait_for_exit();
    Ok(())
}

// === User Input ===
fn prompt_choice<T: Copy>(title: &str, options: &[(&str, T)]) -> Result<T> {
    println!("{}", title);
    for (i, (label, _)) in options.iter().enumerate() {
        println!("{}. {}", (i + 1).to_string().yellow(), label);
    }

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let choice = input
        .trim()
        .parse::<usize>()
        .ok()
        .and_then(|n| n.checked_sub(1))
        .and_then(|i| options.get(i));

    Ok(match choice {
        Some((_, value)) => *value,
        None => {
            println!("Invalid selection, defaulting to {}", options[0].0);
            options[0].1
        }
    })
}

fn choose_build_type(mut packages: Vec<Package>) -> Result<Package> {
    if packages.len() == 1 {
        let package = packages.remove(0);
        println!(
            "\nOnly the {} build is available for this selection.",
            package.build_type
        );
        return Ok(package);
    }

    let options: Vec<(&str, usize)> = packages
        .iter()
        .enumerate()
        .map(|(i, p)| (p.build_type, i))
        .collect();
    let index = prompt_choice(
        "\nChoose a build type (type the number and press Enter):",
        &options,
    )?;
    Ok(packages.swap_remove(index))
}

// === Package Discovery ===

/// Returns the build type of a release asset or CI artifact name (without `.zip`) for `arch`.
///
/// Accepted forms:
/// - `rtx-remix-for-x86-games-release-remix-1.6.1` (RTX Remix release)
/// - `rtx-remix-for-x86-games-1468-f35b86a-release` (CI artifact)
/// - `Remix_Plus_v1.5.1_x86_games_release` (per-architecture Remix Plus release)
/// - `Remix_Plus_v2.0.0` (combined Remix Plus release with x86 and x64 files)
fn package_build_type(name: &str, arch: Arch) -> Option<&'static str> {
    let tokens: Vec<&str> =
        if let Some(rest) = name.strip_prefix(&format!("rtx-remix-for-{}-games-", arch.slug())) {
            rest.split('-').collect()
        } else {
            let tokens: Vec<&str> = name.strip_prefix("Remix_Plus_v")?.split('_').collect();
            if tokens.len() == 1 {
                return matches!(arch, Arch::X86 | Arch::X64).then_some("release");
            }
            if !tokens.contains(&arch.slug()) {
                return None;
            }
            tokens
        };

    if tokens.contains(&"symbols") {
        return None;
    }
    BUILD_TYPES
        .into_iter()
        .find(|build_type| tokens.contains(build_type))
}

/// Keeps one package per build type, ordered as in `BUILD_TYPES`.
fn sort_packages(mut packages: Vec<Package>, source: &str) -> Result<Vec<Package>> {
    packages.sort_by_key(|p| BUILD_TYPES.iter().position(|b| *b == p.build_type));
    packages.dedup_by_key(|p| p.build_type);
    if packages.is_empty() {
        anyhow::bail!("No suitable package found in {}", source);
    }
    Ok(packages)
}

fn find_stable_packages(client: &Client, product: Product, arch: Arch) -> Result<Vec<Package>> {
    println!("{}", "Fetching latest stable release information...".cyan());
    let release = get_json(
        client,
        &format!(
            "https://api.github.com/repos/{}/releases/latest",
            product.release_repo()
        ),
    )?;
    let tag = release["tag_name"].as_str().unwrap_or("latest release");

    let packages = release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|asset| {
            let name = asset["name"].as_str()?;
            let build_type = package_build_type(name.strip_suffix(".zip")?, arch)?;
            Some(Package {
                name: name.to_string(),
                build_type,
                url: asset["browser_download_url"].as_str()?.to_string(),
            })
        })
        .collect();

    sort_packages(packages, &format!("stable release {}", tag))
}

fn find_dev_packages(client: &Client, product: Product, arch: Arch) -> Result<Vec<Package>> {
    println!(
        "{}",
        "Fetching latest development build information...".cyan()
    );
    let repo = product.runtime_repo();
    let runs = get_json(
        client,
        &format!(
            "https://api.github.com/repos/{}/actions/runs?branch={}&status=success&per_page=10",
            repo,
            product.dev_branch()
        ),
    )?;

    // Runs come newest first; skip any whose artifacts have expired or lack this architecture.
    for run in runs["workflow_runs"].as_array().into_iter().flatten() {
        let Some(artifacts_url) = run["artifacts_url"].as_str() else {
            continue;
        };
        let artifacts = get_json(client, artifacts_url)?;
        let packages: Vec<Package> = artifacts["artifacts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|artifact| artifact["expired"] != true)
            .filter_map(|artifact| {
                let name = artifact["name"].as_str()?;
                let build_type = package_build_type(name, arch)?;
                Some(Package {
                    name: name.to_string(),
                    build_type,
                    url: format!(
                        "https://nightly.link/{}/actions/artifacts/{}.zip",
                        repo,
                        artifact["id"].as_u64()?
                    ),
                })
            })
            .collect();
        if !packages.is_empty() {
            return sort_packages(packages, "the latest development build");
        }
    }
    anyhow::bail!("No successful development build with packages for this game type was found")
}

fn get_json(client: &Client, url: &str) -> Result<Value> {
    client
        .get(url)
        .send()
        .and_then(|r| r.error_for_status())
        .with_context(|| format!("Request to {} failed", url))?
        .json()
        .with_context(|| format!("Invalid response from {}", url))
}

// === Install Folder Management ===

/// The folder next to the executable, so the install never depends on the working directory.
fn install_base_dir() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("Failed to locate the downloader")?;
    exe.parent()
        .map(Path::to_path_buf)
        .context("Failed to locate the downloader's folder")
}

/// Creates a new folder named `<prefix>-<time>` that did not exist before.
fn create_unique_dir(base: &Path, prefix: &str) -> Result<PathBuf> {
    let path = unique_path(base, prefix);
    fs::create_dir(&path).with_context(|| format!("Failed to create {}", path.display()))?;
    Ok(path)
}

fn unique_path(base: &Path, prefix: &str) -> PathBuf {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    (0..)
        .map(|n| match n {
            0 => base.join(format!("{}-{}", prefix, secs)),
            n => base.join(format!("{}-{}-{}", prefix, secs, n)),
        })
        .find(|path| fs::symlink_metadata(path).is_err())
        .expect("an unused name always exists")
}

/// Moves the finished install into place. Files of the previous install that
/// the downloader wrote and nobody changed are deleted; anything else in the
/// way is kept in a `remix-old-<time>` folder.
fn replace_install(staging: &Path, install_dir: &Path) -> Result<()> {
    if fs::symlink_metadata(install_dir).is_ok() {
        println!("{}", "Removing the previous installation...".cyan());
        remove_unchanged_installed_files(install_dir).with_context(|| {
            format!(
                "Failed to clear the previous install; the new install is in {}",
                display_path(staging)
            )
        })?;
    }
    if fs::symlink_metadata(install_dir).is_ok() {
        let kept = unique_path(
            install_dir.parent().unwrap_or(install_dir),
            KEPT_FILES_PREFIX,
        );
        fs::rename(install_dir, &kept).with_context(|| {
            format!(
                "Failed to move {} aside; the new install is in {}",
                display_path(install_dir),
                display_path(staging)
            )
        })?;
        println!(
            "{}",
            format!(
                "Files you added or changed in the previous install were kept in {}",
                display_path(&kept)
            )
            .yellow()
        );
    }
    fs::rename(staging, install_dir).with_context(|| {
        format!(
            "Failed to move the new install into place; it is in {}",
            display_path(staging)
        )
    })
}

/// A file's size and modification time, used to tell whether it changed since install.
fn fingerprint(metadata: &fs::Metadata) -> Option<(u64, u128)> {
    let modified = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((metadata.len(), modified.as_nanos()))
}

/// Lists every file of a finished install so a later run knows which files it may delete.
fn write_manifest(dir: &Path) -> Result<()> {
    let mut lines = String::new();
    for path in list_files(dir)? {
        let Some((size, modified)) = fingerprint(&fs::symlink_metadata(&path)?) else {
            continue;
        };
        let relative = path.strip_prefix(dir)?.to_string_lossy().replace('\\', "/");
        lines.push_str(&format!("{}\t{}\t{}\n", size, modified, relative));
    }
    fs::write(dir.join(MANIFEST_NAME), lines)?;
    Ok(())
}

fn list_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            files.extend(list_files(&entry.path())?);
        } else if file_type.is_file() {
            files.push(entry.path());
        }
    }
    Ok(files)
}

/// Deletes the files a previous run installed that are still exactly as installed,
/// then the folders that became empty. Without a manifest nothing is deleted.
fn remove_unchanged_installed_files(dir: &Path) -> Result<()> {
    if !fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir()) {
        return Ok(());
    }
    let manifest_path = dir.join(MANIFEST_NAME);
    let Ok(manifest) = fs::read_to_string(&manifest_path) else {
        return Ok(());
    };

    let mut folders = Vec::new();
    for (size, modified, relative) in manifest.lines().filter_map(parse_manifest_line) {
        let path = dir.join(relative);
        let unchanged = fs::symlink_metadata(&path)
            .is_ok_and(|m| m.is_file() && fingerprint(&m) == Some((size, modified)));
        if unchanged {
            fs::remove_file(&path)
                .with_context(|| format!("Failed to remove {}", path.display()))?;
        }
        folders.extend(
            path.ancestors()
                .skip(1)
                .take_while(|p| *p != dir)
                .map(Path::to_path_buf),
        );
    }
    fs::remove_file(&manifest_path)?;

    // Deepest first; `remove_dir` only removes folders that are empty.
    folders.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    folders.dedup();
    for folder in folders.iter().map(PathBuf::as_path).chain([dir]) {
        let _ = fs::remove_dir(folder);
    }
    Ok(())
}

/// Parses `<size>\t<modified>\t<relative path>`, rejecting paths that could leave the install.
fn parse_manifest_line(line: &str) -> Option<(u64, u128, &Path)> {
    let mut parts = line.splitn(3, '\t');
    let size = parts.next()?.parse().ok()?;
    let modified = parts.next()?.parse().ok()?;
    let relative = Path::new(parts.next()?);
    let contained = relative.components().next().is_some()
        && relative
            .components()
            .all(|c| matches!(c, Component::Normal(_)));
    contained.then_some((size, modified, relative))
}

// === Installation ===

fn install(
    client: &Client,
    dir: &Path,
    product: Product,
    stream: Stream,
    arch: Arch,
    package: &Package,
) -> Result<()> {
    println!(
        "{}",
        format!("\nDownloading {} ({})...", package.name, package.url).cyan()
    );
    download_and_extract(client, &package.url, dir)?;
    flatten_combined_package(dir, arch)?;

    if stream == Stream::Stable {
        remove_debug_files(dir)?;
    }

    if arch == Arch::X86 {
        install_dx8_wrapper(client, dir)?;
        download_if_missing(
            client,
            &dir.join(".trex").join("bridge.conf"),
            &product.raw_url("bridge/bridge.conf"),
        )?;
    }
    download_if_missing(
        client,
        &dir.join("dxvk.conf"),
        &product.raw_url("dxvk.conf"),
    )?;

    // Remix Plus packages carry their own license files.
    if product == Product::Remix {
        let bridge_license = (arch == Arch::X86).then_some(REMIX_BRIDGE_LICENSE);
        for (name, url) in REMIX_LICENSES.into_iter().chain(bridge_license) {
            download_if_missing(client, &dir.join(name), url)?;
        }
    }

    fs::write(dir.join("build-names.txt"), format!("{}\n", package.name))?;
    Ok(())
}

/// Lays out a combined Remix Plus release like a per-architecture package:
/// the game folder as-is for x86 games, or the runtime alone for x64 games.
fn flatten_combined_package(dir: &Path, arch: Arch) -> Result<()> {
    let game_folder = dir.join(COMBINED_GAME_FOLDER);
    if !game_folder.is_dir() {
        return Ok(());
    }

    if arch == Arch::X86 {
        move_dir_contents(&game_folder, dir)?;
    } else {
        let runtime = game_folder.join(".trex");
        move_dir_contents(&runtime, dir)?;
        remove_if_exists(&dir.join("NvRemixBridge.exe"));
        remove_if_exists(&dir.join("bridge.conf"));
    }
    fs::remove_dir_all(&game_folder)?;
    Ok(())
}

fn move_dir_contents(from: &Path, to: &Path) -> Result<()> {
    for entry in fs::read_dir(from)? {
        let source = entry?.path();
        let dest = to.join(source.file_name().unwrap_or_default());
        if dest.is_dir() {
            fs::remove_dir_all(&dest)?;
        } else if dest.exists() {
            fs::remove_file(&dest)?;
        }
        fs::rename(&source, &dest)
            .with_context(|| format!("Failed to move {}", source.display()))?;
    }
    Ok(())
}

/// Installs dxwrapper's d3d8 as `d3d8_off.dll` in place of Remix's d3d8to9.
///
/// The package is staged separately because its `License.txt` would replace
/// the runtime's `LICENSE.txt` on case-insensitive file systems.
fn install_dx8_wrapper(client: &Client, dir: &Path) -> Result<()> {
    println!("{}", "Downloading dx8 binaries...".cyan());
    let staging = dir.join("dx8-wrapper");
    fs::create_dir_all(&staging)?;
    download_and_extract(client, DX8_WRAPPER_URL, &staging)?;

    for (from, to) in DX8_WRAPPER_FILES {
        let source = staging.join(from);
        if source.exists() {
            fs::rename(&source, dir.join(to))?;
        }
    }
    fs::remove_dir_all(&staging)?;
    remove_if_exists(&dir.join("d3d8to9.dll"));
    remove_if_exists(&dir.join("ThirdPartyLicenses-d3d8to9.txt"));
    Ok(())
}

fn download_if_missing(client: &Client, dest: &Path, url: &str) -> Result<()> {
    if dest.exists() {
        return Ok(());
    }
    let name = dest.file_name().unwrap_or_default().to_string_lossy();
    println!("{}", format!("Downloading {}...", name).cyan());
    download_file(client, url, dest)
}

fn download_and_extract(client: &Client, url: &str, dir: &Path) -> Result<()> {
    let zip_path = dir.join("download.zip");
    download_file(client, url, &zip_path)?;

    println!("Extracting package...");
    let file = fs::File::open(&zip_path)?;
    zip::ZipArchive::new(file)
        .and_then(|mut archive| archive.extract(dir))
        .with_context(|| format!("Failed to extract {}", url))?;
    fs::remove_file(zip_path)?;
    Ok(())
}

fn download_file(client: &Client, url: &str, dest: &Path) -> Result<()> {
    let response = client
        .get(url)
        .send()
        .and_then(|r| r.error_for_status())
        .with_context(|| format!("Download of {} failed", url))?;

    let pb = ProgressBar::new(response.content_length().unwrap_or(0));
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")?
            .progress_chars("#>-"),
    );

    let mut file = fs::File::create(dest)?;
    io::copy(&mut pb.wrap_read(response), &mut file)
        .with_context(|| format!("Download of {} was interrupted", url))?;
    file.flush()?;
    pb.finish();
    Ok(())
}

// === File System Helpers ===
fn remove_debug_files(dir: &Path) -> Result<()> {
    let removed = remove_debug_files_recursive(dir)?;
    if removed > 0 {
        println!(
            "{}",
            format!("Cleaned up {} debugging symbol files", removed).cyan()
        );
    }
    Ok(())
}

fn remove_debug_files_recursive(dir: &Path) -> Result<u32> {
    let mut removed = 0;
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            removed += remove_debug_files_recursive(&path)?;
        } else if is_debug_file(&path) && fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

fn is_debug_file(path: &Path) -> bool {
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    path.extension().is_some_and(|ext| ext == "pdb")
        || file_name == "CRC.txt"
        || file_name == "artifacts_readme.txt"
}

fn remove_if_exists(path: &Path) {
    if path.exists() {
        if let Err(e) = fs::remove_file(path) {
            eprintln!(
                "{}",
                format!("Warning: Could not remove {}: {}", path.display(), e).yellow()
            );
        }
    }
}

// === Output ===
fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace(r"\\?\", "")
}

fn display_completion_message(dir: &Path, product: Product) {
    let path = display_path(dir);
    println!("{}", "Download complete!".green().bold());
    println!("Copy the contents of this folder into the folder with the game's .exe:");
    println!(
        "{}",
        format!("\x1B]8;;file://{}\x07{}\x1B]8;;\x07", path, path).cyan()
    );
    let guide = match product {
        Product::Remix => "https://github.com/NVIDIAGameWorks/rtx-remix/wiki/runtime-user-guide",
        Product::RemixPlus => "https://github.com/RemixProjGroup/dxvk-remix",
    };
    println!("{}", "Install guide:".yellow());
    println!("{}", guide.cyan());
}

fn wait_for_exit() {
    println!("\nPress Enter to exit...");
    let mut input = String::new();
    let _ = io::stdin().read_line(&mut input);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remix_release_names() {
        let name = "rtx-remix-for-x86-games-debugoptimized-remix-1.6.1";
        assert_eq!(package_build_type(name, Arch::X86), Some("debugoptimized"));
        assert_eq!(package_build_type(name, Arch::X64), None);
        assert_eq!(
            package_build_type("rtx-remix-for-arm64-games-debug-remix-1.6.1", Arch::Arm64),
            Some("debug")
        );
        assert_eq!(
            package_build_type(
                "rtx-remix-for-x64-games-release-remix-1.6.1-symbols",
                Arch::X64
            ),
            None
        );
    }

    #[test]
    fn ci_artifact_names() {
        let name = "rtx-remix-for-arm64-games-1468-f35b86a-release";
        assert_eq!(package_build_type(name, Arch::Arm64), Some("release"));
        assert_eq!(package_build_type(name, Arch::X64), None);
        assert_eq!(
            package_build_type("build-logs-1468-f35b86a-release", Arch::X64),
            None
        );
    }

    #[test]
    fn manifest_paths_stay_inside_install() {
        assert!(parse_manifest_line("1\t2\t.trex/d3d9.dll").is_some());
        for bad in [
            "../x.dll",
            "a/../../x.dll",
            "/x.dll",
            "C:/x.dll",
            r"C:\x.dll",
            "",
        ] {
            assert!(
                parse_manifest_line(&format!("1\t2\t{}", bad)).is_none(),
                "{}",
                bad
            );
        }
    }

    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "remix-downloader-test-{}-{}",
            name,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn kept_dirs(base: &Path) -> Vec<PathBuf> {
        fs::read_dir(base)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(KEPT_FILES_PREFIX)
            })
            .collect()
    }

    #[test]
    fn reinstall_keeps_user_files() {
        let base = test_dir("keep");
        let old = base.join(INSTALL_DIR);
        fs::create_dir_all(old.join(".trex")).unwrap();
        fs::write(old.join("d3d9.dll"), "old").unwrap();
        fs::write(old.join(".trex").join("d3d9.dll"), "old").unwrap();
        fs::write(old.join("dxvk.conf"), "old").unwrap();
        write_manifest(&old).unwrap();
        fs::write(old.join("dxvk.conf"), "edited by the user").unwrap();
        fs::write(old.join(".trex").join("rtx.conf"), "user").unwrap();

        let staging = create_unique_dir(&base, STAGING_PREFIX).unwrap();
        fs::write(staging.join("d3d9.dll"), "new").unwrap();
        replace_install(&staging, &old).unwrap();

        assert_eq!(fs::read_to_string(old.join("d3d9.dll")).unwrap(), "new");
        let kept = kept_dirs(&base);
        assert_eq!(kept.len(), 1);
        let mut files: Vec<String> = list_files(&kept[0])
            .unwrap()
            .iter()
            .map(|p| {
                p.strip_prefix(&kept[0])
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        files.sort();
        assert_eq!(files, [".trex/rtx.conf", "dxvk.conf"]);
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn foreign_folder_is_moved_aside_untouched() {
        let base = test_dir("foreign");
        let old = base.join(INSTALL_DIR);
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("notes.txt"), "mine").unwrap();
        fs::write(old.join("d3d9.dll"), "mine").unwrap();

        let staging = create_unique_dir(&base, STAGING_PREFIX).unwrap();
        fs::write(staging.join("d3d9.dll"), "new").unwrap();
        replace_install(&staging, &old).unwrap();

        let kept = kept_dirs(&base);
        assert_eq!(kept.len(), 1);
        assert_eq!(
            fs::read_to_string(kept[0].join("notes.txt")).unwrap(),
            "mine"
        );
        assert_eq!(
            fs::read_to_string(kept[0].join("d3d9.dll")).unwrap(),
            "mine"
        );
        assert_eq!(fs::read_to_string(old.join("d3d9.dll")).unwrap(), "new");
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn remix_plus_release_names() {
        assert_eq!(
            package_build_type("Remix_Plus_v2.0.0", Arch::X64),
            Some("release")
        );
        assert_eq!(package_build_type("Remix_Plus_v2.0.0", Arch::Arm64), None);
        let name = "Remix_Plus_v1.5.1_x86_games_debugoptimized";
        assert_eq!(package_build_type(name, Arch::X86), Some("debugoptimized"));
        assert_eq!(package_build_type(name, Arch::X64), None);
    }
}
