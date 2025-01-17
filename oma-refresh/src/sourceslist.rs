use std::path::Path;

use ahash::HashMap;
use apt_auth_config::{AuthConfig, Authenticator};
use oma_apt_sources_lists::{
    Signature, SourceEntry, SourceLine, SourceListType, SourcesList, SourcesListError,
};
use once_cell::sync::OnceCell;
use url::Url;

use crate::{
    db::{Event, RefreshError},
    util::DatabaseFilenameReplacer,
};
use std::future::Future;

#[derive(Debug, Clone)]
pub struct OmaSourceEntry<'a> {
    source: SourceEntry,
    arch: &'a str,
    url: OnceCell<String>,
    suite: OnceCell<String>,
    dist_path: OnceCell<String>,
    from: OnceCell<OmaSourceEntryFrom>,
}

pub async fn sources_lists<F, Fut>(
    sysroot: impl AsRef<Path>,
    arch: &str,
    cb: F,
) -> Result<Vec<OmaSourceEntry<'_>>, SourcesListError>
where
    F: Fn(Event) -> Fut,
    Fut: Future<Output = ()>,
{
    let mut res = Vec::new();
    let mut paths = vec![];
    let default = sysroot.as_ref().join("etc/apt/sources.list");

    if default.exists() {
        paths.push(default);
    }

    if sysroot.as_ref().join("etc/apt/sources.list.d/").exists() {
        let mut dir = tokio::fs::read_dir(sysroot.as_ref().join("etc/apt/sources.list.d/")).await?;
        while let Some(entry) = dir.next_entry().await? {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            paths.push(path);
        }
    }

    for p in paths {
        match SourcesList::new(p) {
            Ok(s) => match s.entries {
                SourceListType::SourceLine(source_list_line_style) => {
                    for i in source_list_line_style.0 {
                        if let SourceLine::Entry(entry) = i {
                            res.push(OmaSourceEntry::new(entry, arch));
                        }
                    }
                }
                SourceListType::Deb822(source_list_deb822) => {
                    for i in source_list_deb822.entries {
                        res.push(OmaSourceEntry::new(i, arch));
                    }
                }
            },
            Err(e) => match e {
                SourcesListError::UnknownFile { path } => {
                    cb(Event::SourceListFileNotSupport { path }).await;
                }
                e => return Err(e),
            },
        }
    }

    Ok(res)
}

#[derive(PartialEq, Eq, Debug, Copy, Clone)]
pub enum OmaSourceEntryFrom {
    Http,
    Local,
}

impl<'a> OmaSourceEntry<'a> {
    pub fn new(source: SourceEntry, arch: &'a str) -> Self {
        Self {
            source,
            arch,
            url: OnceCell::new(),
            suite: OnceCell::new(),
            dist_path: OnceCell::new(),
            from: OnceCell::new(),
        }
    }

    pub fn from(&self) -> Result<&OmaSourceEntryFrom, RefreshError> {
        self.from.get_or_try_init(|| {
            let url = self.source.url();
            if url.starts_with("http") {
                Ok(OmaSourceEntryFrom::Http)
            } else if url.starts_with("file") {
                Ok(OmaSourceEntryFrom::Local)
            } else {
                return Err(RefreshError::UnsupportedProtocol(url.to_string()));
            }
        })
    }

    pub fn components(&self) -> &[String] {
        &self.source.components
    }

    pub fn archs(&self) -> &Option<Vec<String>> {
        &self.source.archs
    }

    pub fn trusted(&self) -> bool {
        self.source.trusted
    }

    pub fn signed_by(&self) -> &Option<Signature> {
        &self.source.signed_by
    }

    pub fn url(&self) -> &str {
        self.url
            .get_or_init(|| self.source.url.replace("$(ARCH)", self.arch))
    }

    pub fn is_flat(&self) -> bool {
        self.components().is_empty()
    }

    pub fn suite(&self) -> &str {
        self.suite
            .get_or_init(|| self.source.suite.replace("$(ARCH)", self.arch))
    }

    pub fn is_source(&self) -> bool {
        self.source.source
    }

    pub fn dist_path(&self) -> &str {
        self.dist_path.get_or_init(|| {
            let suite = self.suite();
            let url = self.url();

            if self.is_flat() {
                if suite == "/" {
                    if !url.ends_with('/') {
                        format!("{}{}", url, suite)
                    } else {
                        url.to_string()
                    }
                } else if url.ends_with('/') {
                    format!("{}{}", url, suite)
                } else {
                    format!("{}/{}", url, suite)
                }
            } else {
                self.source.dist_path()
            }
        })
    }

    pub fn get_human_download_url(&self, file_name: Option<&str>) -> Result<String, RefreshError> {
        let url = self.url();
        let url = Url::parse(url).map_err(|_| RefreshError::InvalidUrl(url.to_string()))?;

        let host = url.host_str();

        let url = if let Some(host) = host {
            host
        } else {
            url.path()
        };

        let mut s = format!("{}:{}", url, self.suite());

        if let Some(file_name) = file_name {
            s.push(' ');
            s.push_str(file_name);
        }

        Ok(s)
    }
}

#[derive(Debug)]
pub(crate) struct MirrorSources<'a, 'b>(pub Vec<MirrorSource<'a, 'b>>);

#[derive(Debug)]
pub(crate) struct MirrorSource<'a, 'b> {
    pub(crate) sources: Vec<&'a OmaSourceEntry<'a>>,
    release_file_name: OnceCell<String>,
    auth: Option<&'b Authenticator>,
}

impl MirrorSource<'_, '_> {
    pub(crate) fn set_release_file_name(&self, file_name: String) {
        self.release_file_name
            .set(file_name)
            .expect("Release file name was init");
    }

    pub(crate) fn dist_path(&self) -> &str {
        self.sources.first().unwrap().dist_path()
    }

    #[cfg(feature = "aosc")]
    pub(crate) fn suite(&self) -> &str {
        self.sources.first().unwrap().suite()
    }

    pub(crate) fn from(&self) -> Result<&OmaSourceEntryFrom, RefreshError> {
        self.sources.first().unwrap().from()
    }

    pub(crate) fn get_human_download_url(
        &self,
        file_name: Option<&str>,
    ) -> Result<String, RefreshError> {
        self.sources
            .first()
            .unwrap()
            .get_human_download_url(file_name)
    }

    pub(crate) fn signed_by(&self) -> Option<&Signature> {
        self.sources.iter().find_map(|x| {
            if let Some(x) = &x.signed_by() {
                Some(x)
            } else {
                None
            }
        })
    }

    pub(crate) fn url(&self) -> &str {
        self.sources.first().unwrap().url()
    }

    pub(crate) fn is_flat(&self) -> bool {
        self.sources.first().unwrap().is_flat()
    }

    pub(crate) fn trusted(&self) -> bool {
        self.sources.iter().any(|x| x.trusted())
    }

    pub(crate) fn file_name(&self) -> Option<&str> {
        self.release_file_name.get().map(|x| x.as_str())
    }

    pub(crate) fn auth(&self) -> Option<&Authenticator> {
        self.auth
    }
}

impl<'a, 'b> MirrorSources<'a, 'b> {
    pub(crate) fn from_sourcelist(
        sourcelist: &'a [OmaSourceEntry<'a>],
        replacer: &DatabaseFilenameReplacer,
        auth_config: Option<&'b AuthConfig>,
    ) -> Result<Self, RefreshError> {
        let mut map: HashMap<String, Vec<&OmaSourceEntry>> =
            HashMap::with_hasher(ahash::RandomState::new());

        if sourcelist.is_empty() {
            return Err(RefreshError::SourceListsEmpty);
        }

        for source in sourcelist {
            let dist_path = source.dist_path();
            let name = replacer.replace(dist_path)?;

            map.entry(name).or_default().push(source);
        }

        let mut res = vec![];

        for (_, v) in map {
            let url = v[0].url();
            let auth = auth_config.and_then(|auth| auth.find(url));

            res.push(MirrorSource {
                sources: v,
                release_file_name: OnceCell::new(),
                auth,
            });
        }

        Ok(Self(res))
    }
}

#[test]
fn test_ose() {
    use oma_utils::dpkg::dpkg_arch;
    // Flat repository tests.

    // deb file:///debs/ /
    let entry = SourceEntry {
        enabled: true,
        source: false,
        options: vec![],
        url: "file:///debs/".to_string(),
        suite: "/".to_string(),
        components: vec![],
        is_deb822: false,
        archs: None,
        signed_by: None,
        trusted: false,
    };

    let arch = dpkg_arch("/").unwrap();
    let ose = OmaSourceEntry::new(entry, &arch);
    assert_eq!(ose.url(), "file:///debs/");
    assert_eq!(ose.dist_path(), "file:///debs/");

    // deb file:///debs/ ./
    let entry = SourceEntry {
        enabled: true,
        source: false,
        options: vec![],
        url: "file:///debs/".to_string(),
        suite: "./".to_string(),
        components: vec![],
        is_deb822: false,
        archs: None,
        signed_by: None,
        trusted: false,
    };

    let arch = dpkg_arch("/").unwrap();
    let ose = OmaSourceEntry::new(entry, &arch);
    assert_eq!(ose.url(), "file:///debs/");
    assert_eq!(ose.dist_path(), "file:///debs/./");

    // deb file:/debs/ /
    let entry = SourceEntry {
        enabled: true,
        source: false,
        options: vec![],
        url: "file:/debs/".to_string(),
        suite: "/".to_string(),
        components: vec![],
        is_deb822: false,
        archs: None,
        signed_by: None,
        trusted: false,
    };

    let arch = dpkg_arch("/").unwrap();
    let ose = OmaSourceEntry::new(entry, &arch);
    assert_eq!(ose.url(), "file:/debs/");
    assert_eq!(ose.dist_path(), "file:/debs/");

    // deb file:/debs /
    //
    // APT will append implicitly a / at the end of the URL.
    let entry = SourceEntry {
        enabled: true,
        source: false,
        options: vec![],
        url: "file:/debs".to_string(),
        suite: "/".to_string(),
        components: vec![],
        is_deb822: false,
        archs: None,
        signed_by: None,
        trusted: false,
    };

    let arch = dpkg_arch("/").unwrap();
    let ose = OmaSourceEntry::new(entry, &arch);
    assert_eq!(ose.url(), "file:/debs");
    assert_eq!(ose.dist_path(), "file:/debs/");

    // deb file:/debs/ ./././
    let entry = SourceEntry {
        enabled: true,
        source: false,
        options: vec![],
        url: "file:/debs/".to_string(),
        suite: "./././".to_string(),
        components: vec![],
        is_deb822: false,
        archs: None,
        signed_by: None,
        trusted: false,
    };

    let arch = dpkg_arch("/").unwrap();
    let ose = OmaSourceEntry::new(entry, &arch);
    assert_eq!(ose.url(), "file:/debs/");
    assert_eq!(ose.dist_path(), "file:/debs/./././");

    // deb file:/debs/ .//
    //
    // APT will throw a warning but carry on with the suite name:
    //
    // W: Conflicting distribution: file:/debs .// Release (expected .// but got )
    let entry = SourceEntry {
        enabled: true,
        source: false,
        options: vec![],
        url: "file:/debs/".to_string(),
        suite: ".//".to_string(),
        components: vec![],
        is_deb822: false,
        archs: None,
        signed_by: None,
        trusted: false,
    };

    let arch = dpkg_arch("/").unwrap();
    let ose = OmaSourceEntry::new(entry, &arch);
    assert_eq!(ose.url(), "file:/debs/");
    assert_eq!(ose.dist_path(), "file:/debs/.//");

    // deb file:/debs/ //
    //
    // APT will throw a warning but carry on with the suite name:
    //
    // W: Conflicting distribution: file:/debs // Release (expected // but got )
    let entry = SourceEntry {
        enabled: true,
        source: false,
        options: vec![],
        url: "file:/debs/".to_string(),
        suite: "//".to_string(),
        components: vec![],
        is_deb822: false,
        archs: None,
        signed_by: None,
        trusted: false,
    };

    let arch = dpkg_arch("/").unwrap();
    let ose = OmaSourceEntry::new(entry, &arch);
    assert_eq!(ose.url(), "file:/debs/");
    assert_eq!(ose.dist_path(), "file:/debs///");

    // deb file:/./debs/ ./
    let entry = SourceEntry {
        enabled: true,
        source: false,
        options: vec![],
        url: "file:/./debs/".to_string(),
        suite: "./".to_string(),
        components: vec![],
        is_deb822: false,
        archs: None,
        signed_by: None,
        trusted: false,
    };

    let arch = dpkg_arch("/").unwrap();
    let ose = OmaSourceEntry::new(entry, &arch);
    assert_eq!(ose.url(), "file:/./debs/");
    assert_eq!(ose.dist_path(), "file:/./debs/./");

    // deb file:/usr/../debs/ ./
    let entry = SourceEntry {
        enabled: true,
        source: false,
        options: vec![],
        url: "file:/usr/../debs/".to_string(),
        suite: "./".to_string(),
        components: vec![],
        is_deb822: false,
        archs: None,
        signed_by: None,
        trusted: false,
    };

    let arch = dpkg_arch("/").unwrap();
    let ose = OmaSourceEntry::new(entry, &arch);
    assert_eq!(ose.url(), "file:/usr/../debs/");
    assert_eq!(ose.dist_path(), "file:/usr/../debs/./");
}

#[test]
fn test_database_filename() {
    use crate::util::DatabaseFilenameReplacer;
    let replacer = DatabaseFilenameReplacer::new().unwrap();

    // Encode + as %252b.
    let s = "https://repo.aosc.io/debs/dists/x264-0+git20240305/InRelease";
    let res = replacer.replace(s).unwrap();
    assert_eq!(
        res,
        "repo.aosc.io_debs_dists_x264-0%252bgit20240305_InRelease"
    );

    // Encode : as %3A.
    let s = "https://ci.deepin.com/repo/obs/deepin%3A/CI%3A/TestingIntegration%3A/test-integration-pr-1537/testing/./Packages";
    let res = replacer.replace(s).unwrap();
    assert_eq!(res, "ci.deepin.com_repo_obs_deepin:_CI:_TestingIntegration:_test-integration-pr-1537_testing_._Packages");

    // Encode _ as %5f
    let s = "https://repo.aosc.io/debs/dists/xorg-server-21.1.13-hyperv_drm-fix";
    let res = replacer.replace(s).unwrap();
    assert_eq!(
        res,
        "repo.aosc.io_debs_dists_xorg-server-21.1.13-hyperv%5fdrm-fix"
    );

    // file:/// should be transliterated as file:/.
    let s1 = "file:/debs";
    let s2 = "file:///debs";
    let res1 = replacer.replace(s1).unwrap();
    let res2 = replacer.replace(s2).unwrap();
    assert_eq!(res1, "_debs");
    assert_eq!(res1, res2);

    // Dots (.) in flat repo URLs should be preserved in resolved database name.
    let s = "file:///././debs/./Packages";
    let res = replacer.replace(s).unwrap();
    assert_eq!(res, "_._._debs_._Packages");

    // Slash (/) in flat repo "suite" names should be transliterated as _.
    let s = "file:///debs/Packages";
    let res = replacer.replace(s).unwrap();
    assert_eq!(res, "_debs_Packages");

    // Dots (.) in flat repo "suite" names should be preserved in resolved database name.
    let s = "file:///debs/./Packages";
    let res = replacer.replace(s).unwrap();
    assert_eq!(res, "_debs_._Packages");

    // Slashes in URL and in flat repo "suite" names should be preserved in original number (1).
    let s = "file:///debs///./Packages";
    let res = replacer.replace(s).unwrap();
    assert_eq!(res, "_debs___._Packages");

    // Slashes in URL and in flat repo "suite" names should be preserved in original number (2).
    let s = "file:///debs///.///Packages";
    let res = replacer.replace(s).unwrap();
    assert_eq!(res, "_debs___.___Packages");
}
