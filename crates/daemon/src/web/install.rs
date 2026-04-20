use super::*;

pub(super) fn default_web_install_dir() -> PathBuf {
    mvp::config::default_loong_home().join("web")
}

pub(super) fn web_install_dist_dir(install_dir: &FsPath) -> PathBuf {
    install_dir.join("dist")
}

fn web_install_manifest_path(install_dir: &FsPath) -> PathBuf {
    install_dir.join("install.json")
}

fn copy_dir_all(src: &FsPath, dst: &FsPath) -> CliResult<()> {
    for entry in
        fs::read_dir(src).map_err(|error| format!("failed to read `{}`: {error}", src.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            fs::create_dir_all(&dst_path)
                .map_err(|error| format!("failed to create `{}`: {error}", dst_path.display()))?;
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path).map_err(|error| {
                format!(
                    "failed to copy `{}` to `{}`: {error}",
                    src_path.display(),
                    dst_path.display()
                )
            })?;
        }
    }
    Ok(())
}

pub(super) fn run_web_install(source: &str) -> CliResult<()> {
    let source_path = PathBuf::from(source);
    if !source_path.exists() {
        return Err(format!(
            "source path `{}` does not exist",
            source_path.display()
        ));
    }
    if !source_path.is_dir() {
        return Err(format!(
            "source path `{}` is not a directory",
            source_path.display()
        ));
    }
    if !source_path.join("index.html").is_file() {
        return Err(format!(
            "source path `{}` is missing `index.html` — run `npm run build` first",
            source_path.display()
        ));
    }

    let install_dir = default_web_install_dir();
    let dist_dir = web_install_dist_dir(&install_dir);
    let staging_dir = install_dir.join(format!("dist.staging-{}", random::<u64>()));
    let backup_dir = install_dir.join(format!("dist.backup-{}", random::<u64>()));

    fs::create_dir_all(&install_dir).map_err(|error| {
        format!(
            "failed to create install root `{}`: {error}",
            install_dir.display()
        )
    })?;

    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir).map_err(|error| {
            format!(
                "failed to clear staging install `{}`: {error}",
                staging_dir.display()
            )
        })?;
    }
    fs::create_dir_all(&staging_dir).map_err(|error| {
        format!(
            "failed to create staging install directory `{}`: {error}",
            staging_dir.display()
        )
    })?;

    if let Err(error) = copy_dir_all(&source_path, &staging_dir) {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(error);
    }

    let promote_result: CliResult<()> = (|| {
        if dist_dir.exists() {
            if backup_dir.exists() {
                fs::remove_dir_all(&backup_dir).map_err(|error| {
                    format!(
                        "failed to clear previous backup install `{}`: {error}",
                        backup_dir.display()
                    )
                })?;
            }
            fs::rename(&dist_dir, &backup_dir).map_err(|error| {
                format!(
                    "failed to stage existing install `{}`: {error}",
                    dist_dir.display()
                )
            })?;
        }

        if let Err(error) = fs::rename(&staging_dir, &dist_dir) {
            if backup_dir.exists() && !dist_dir.exists() {
                let _ = fs::rename(&backup_dir, &dist_dir);
            }
            return Err(format!(
                "failed to promote staged install `{}`: {error}",
                staging_dir.display()
            ));
        }

        if backup_dir.exists() {
            fs::remove_dir_all(&backup_dir).map_err(|error| {
                format!(
                    "failed to remove previous install backup `{}`: {error}",
                    backup_dir.display()
                )
            })?;
        }
        Ok(())
    })();

    if let Err(error) = promote_result {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(error);
    }

    let canonical_source = source_path
        .canonicalize()
        .unwrap_or_else(|_| source_path.clone());
    let manifest = WebInstallManifest {
        installed_at: OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_default(),
        source_path: canonical_source.display().to_string(),
        install_dir: install_dir.display().to_string(),
    };
    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|error| format!("failed to serialize install manifest: {error}"))?;
    fs::write(web_install_manifest_path(&install_dir), manifest_json)
        .map_err(|error| format!("failed to write install manifest: {error}"))?;

    println!("Web Console installed to: {}", dist_dir.display());
    println!("Run `loong web serve` to start the same-origin Web Console.");
    Ok(())
}

pub(super) fn run_web_status() -> CliResult<()> {
    let install_dir = default_web_install_dir();
    let manifest_path = web_install_manifest_path(&install_dir);
    let dist_dir = web_install_dist_dir(&install_dir);

    if !manifest_path.exists() {
        println!("Web Console: not installed");
        println!("Run `loong web install --source <path/to/web/dist>` to install.");
        return Ok(());
    }

    let manifest_raw = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("failed to read install manifest: {error}"))?;
    let manifest: WebInstallManifest = serde_json::from_str(&manifest_raw)
        .map_err(|error| format!("failed to parse install manifest: {error}"))?;

    let assets_ok = dist_dir.join("index.html").is_file();
    println!("Web Console: installed");
    println!("Install dir:  {}", manifest.install_dir);
    println!("Installed at: {}", manifest.installed_at);
    println!("Source:       {}", manifest.source_path);
    println!(
        "Assets:       {}",
        if assets_ok {
            "ok"
        } else {
            "missing (dist/index.html not found — re-run `web install`)"
        }
    );
    Ok(())
}

pub(super) fn run_web_remove(force: bool) -> CliResult<()> {
    let install_dir = default_web_install_dir();
    let manifest_path = web_install_manifest_path(&install_dir);
    let dist_dir = web_install_dist_dir(&install_dir);

    if !manifest_path.exists() && !dist_dir.exists() {
        println!("Web Console: not installed, nothing to remove.");
        return Ok(());
    }

    if !force {
        println!("This will remove: {}", install_dir.display());
        println!("Re-run with --force to confirm removal.");
        return Ok(());
    }

    if dist_dir.exists() {
        fs::remove_dir_all(&dist_dir)
            .map_err(|error| format!("failed to remove `{}`: {error}", dist_dir.display()))?;
    }
    if manifest_path.exists() {
        fs::remove_file(&manifest_path)
            .map_err(|error| format!("failed to remove `{}`: {error}", manifest_path.display()))?;
    }

    println!("Web Console removed from: {}", install_dir.display());
    Ok(())
}
