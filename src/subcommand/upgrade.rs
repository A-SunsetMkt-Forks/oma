use crate::subcommand::utils::CommitChanges;
use crate::utils::pkgnames_and_path_completions;
use clap_complete::ArgValueCompleter;
use std::path::PathBuf;
use tracing::debug;

use apt_auth_config::AuthConfig;
use clap::Args;
use oma_pm::apt::AptConfig;
use oma_pm::apt::OmaApt;
use oma_pm::apt::OmaAptArgs;
use oma_pm::apt::Upgrade as AptUpgrade;

use oma_pm::matches::GetArchMethod;
use oma_pm::matches::PackagesMatcher;

use tracing::info;
use tracing::warn;

use crate::HTTP_CLIENT;
use crate::config::Config;
use crate::error::OutputError;
use crate::fl;
use crate::utils::dbus_check;
use crate::utils::root;

use super::utils::Refresh;
use super::utils::handle_no_result;
use super::utils::lock_oma;
use crate::args::CliExecuter;

#[derive(Debug, Args)]
pub(crate) struct Upgrade {
    /// Do not fix apt broken status
    #[arg(long)]
    no_fixbroken: bool,
    /// Do not fix dpkg broken status
    #[arg(long)]
    no_fix_dpkg_status: bool,
    /// Install package(s) without fsync(2)
    #[arg(long)]
    force_unsafe_io: bool,
    /// Do not refresh repository metadata
    #[arg(long)]
    no_refresh: bool,
    /// Ignore repository and package dependency issues
    #[arg(long)]
    force_yes: bool,
    /// Replace configuration file(s) in the system those shipped in the package(s) to be installed (invokes `dpkg --force-confnew`)
    #[arg(long)]
    force_confnew: bool,
    #[cfg(feature = "aosc")]
    /// Do not refresh topics manifest.json file
    #[arg(long)]
    no_refresh_topics: bool,
    /// Auto remove unnecessary package(s)
    #[arg(long)]
    autoremove: bool,
    /// Remove package(s) also remove configuration file(s), like apt purge
    #[arg(long, visible_alias = "purge")]
    remove_config: bool,
    /// Bypass confirmation prompts
    #[arg(short, long)]
    yes: bool,
    #[cfg(not(feature = "aosc"))]
    /// Do not allow removal of packages during upgrade (like `apt upgrade')
    #[arg(long)]
    no_remove: bool,
    /// Package(s) to install
    #[arg(add = ArgValueCompleter::new(pkgnames_and_path_completions))]
    packages: Vec<String>,
    /// Run oma in "dry-run" mode. Useful for testing changes and operations without making changes to the system
    #[arg(from_global)]
    dry_run: bool,
    /// Run oma do not check dbus
    #[arg(from_global)]
    no_check_dbus: bool,
    /// Set sysroot target directory
    #[arg(from_global)]
    sysroot: PathBuf,
    /// Set apt options
    #[arg(from_global)]
    apt_options: Vec<String>,
    /// Setup download threads (default as 4)
    #[arg(from_global)]
    download_threads: Option<usize>,
    /// Run oma do not check battery status
    #[arg(from_global)]
    no_check_battery: bool,
    /// Run oma do not check battery status
    #[arg(from_global)]
    no_take_wake_lock: bool,
}

impl CliExecuter for Upgrade {
    fn execute(self, config: &Config, no_progress: bool) -> Result<i32, OutputError> {
        let Upgrade {
            no_fixbroken,
            force_unsafe_io,
            no_refresh,
            force_yes,
            force_confnew,
            #[cfg(feature = "aosc")]
            no_refresh_topics,
            autoremove,
            remove_config,
            yes,
            packages,
            dry_run,
            no_check_dbus,
            sysroot,
            apt_options,
            #[cfg(not(feature = "aosc"))]
            no_remove,
            no_fix_dpkg_status,
            download_threads,
            no_check_battery,
            no_take_wake_lock,
        } = self;

        if !dry_run {
            root()?;
            lock_oma()?;
        }

        let _fds = dbus_check(
            false,
            config,
            no_check_dbus,
            dry_run,
            no_take_wake_lock,
            no_check_battery,
        )?;

        let apt_config = AptConfig::new();

        let auth_config = AuthConfig::system(&sysroot)?;

        if !no_refresh {
            let sysroot = sysroot.to_string_lossy();
            let builder = Refresh::builder()
                .client(&HTTP_CLIENT)
                .dry_run(dry_run)
                .no_progress(no_progress)
                .network_thread(download_threads.unwrap_or_else(|| config.network_thread()))
                .sysroot(&sysroot)
                .config(&apt_config)
                .auth_config(&auth_config);

            #[cfg(feature = "aosc")]
            let refresh = builder
                .refresh_topics(!no_refresh_topics && !config.no_refresh_topics())
                .build();

            #[cfg(not(feature = "aosc"))]
            let refresh = builder.build();

            refresh.run()?;
        }

        if yes {
            warn!("{}", fl!("automatic-mode-warn"));
        }

        let local_debs = packages
            .iter()
            .filter(|x| x.ends_with(".deb"))
            .map(|x| x.to_owned())
            .collect::<Vec<_>>();

        let pkgs_unparse = packages.iter().map(|x| x.as_str()).collect::<Vec<_>>();

        let oma_apt_args = OmaAptArgs::builder()
            .sysroot(sysroot.to_string_lossy().to_string())
            .dpkg_force_confnew(force_confnew)
            .force_yes(force_yes)
            .yes(yes)
            .another_apt_options(apt_options)
            .dpkg_force_unsafe_io(force_unsafe_io)
            .build();

        let mut apt = OmaApt::new(local_debs, oma_apt_args, dry_run, AptConfig::new())?;

        #[cfg(feature = "aosc")]
        let mode = AptUpgrade::FullUpgrade;

        #[cfg(not(feature = "aosc"))]
        let mode = if no_remove {
            AptUpgrade::Upgrade
        } else {
            AptUpgrade::FullUpgrade
        };

        debug!("Upgrade mode is using: {:?}", mode);
        apt.upgrade(mode)?;

        let matcher = PackagesMatcher::builder()
            .cache(&apt.cache)
            .filter_candidate(true)
            .filter_downloadable_candidate(false)
            .select_dbg(false)
            .native_arch(GetArchMethod::SpecifySysroot(&sysroot))
            .build();

        let (pkgs, no_result) = matcher.match_pkgs_and_versions(pkgs_unparse.clone())?;

        handle_no_result(&sysroot, no_result, no_progress)?;

        let no_marked_install = apt.install(&pkgs, false)?;

        if !no_marked_install.is_empty() {
            for (pkg, version) in no_marked_install {
                info!(
                    "{}",
                    fl!("already-installed", name = pkg, version = version)
                );
            }
        }

        CommitChanges::builder()
            .apt(apt)
            .dry_run(dry_run)
            .no_fixbroken(no_fixbroken)
            .check_update(true)
            .no_progress(no_progress)
            .sysroot(sysroot.to_string_lossy().to_string())
            .protect_essential(config.protect_essentials())
            .yes(yes)
            .remove_config(remove_config)
            .autoremove(autoremove)
            .network_thread(download_threads.unwrap_or_else(|| config.network_thread()))
            .maybe_auth_config(Some(&auth_config))
            .fix_dpkg_status(!no_fix_dpkg_status)
            .build()
            .run()
    }
}
