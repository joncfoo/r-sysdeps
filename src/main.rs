use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
#[structopt(name = "action")]
enum Action {
    /// Get system dependencies for R packages
    #[structopt(name = "package")]
    Package {
        /// Repository name (case-sensitive)
        #[structopt(short, long, default_value = "all")]
        repository: String,

        /// R packages
        #[structopt()]
        packages: Vec<String>,
    },

    /// Get repository information
    #[structopt(name = "repository")]
    Repository {
        /// List all R repositories on server
        #[structopt(short, long = "list-repositories")]
        list: bool,

        /// Repository name (case-sensitive)
        #[structopt(short, long, default_value = "all")]
        repository: String,

        /// Print binary package URL for repository
        #[structopt(short, long)]
        binary_repository: bool,

        /// Print source package URL for repository
        #[structopt(short, long)]
        source_repository: bool,
    },
}

#[derive(StructOpt, Debug)]
#[structopt(name = "r-sysdeps")]
struct Opt {
    /// Operating System name [auto-detected]
    #[structopt(long = "os-name")]
    os_name: Option<String>,

    /// Operating System version [auto-detected]
    #[structopt(long = "os-version")]
    os_version: Option<String>,

    /// RStudio Package Manager Server
    #[structopt(long = "server", default_value = "https://packagemanager.rstudio.com")]
    server: String,

    /// Action
    #[structopt(subcommand)]
    action: Action,
}

#[derive(Debug, Deserialize)]
struct APIStatusResponse {
    version: String,
    build_date: String,
    r_configured: bool,
    binaries_enabled: bool,
    distros: Vec<APIDistribution>,
    cran_repo: String,
    bioc_versions: Vec<APIBioConductorVersion>,
}

#[derive(Debug, Deserialize)]
struct APIDistribution {
    #[serde(rename = "binaryDisplay")]
    binary_display: String,
    #[serde(rename = "binaryURL")]
    binary_url: String,
    display: String,
    distribution: String,
    release: String,
    #[serde(rename = "sysReqs")]
    sys_reqs: bool,
    binaries: bool,
}

#[derive(Debug, Deserialize)]
struct APIBioConductorVersion {
    bioc_version: String,
    r_version: String,
    cran_snapshot: String,
}

fn main() {
    let opt = Opt::from_args();

    match detect_os() {
        Some(os) => println!("OS = {}", os),
        None => println!("Unsupported OS")
    }

    match server_status(opt.server) {
        Ok(response) => println!("{:#?}", response),
        err => eprintln!("error: {:#?}", err),
    }
}

fn server_status(server: String) -> Result<APIStatusResponse> {
    let http_response = minreq::get(format!("{}/__api__/status", server))
        .with_timeout(10)
        .send()
        .with_context(|| format!("failed to reach server {}", server))?;

    if http_response.status_code < 200 || http_response.status_code > 299 {
        bail!(format!("failed to reach {}/__api__/status", server));
    }

    let api_response = http_response.json()
        .with_context(|| format!("failed to parse JSON response from {}/__api__/status", server))?;

    Ok(api_response)
}

fn detect_os() -> Option<String> {
    let known_os = vec![
        "ubuntu-16.04",
        "ubuntu-18.04",
        "ubuntu-20.04",
        "centos-7",
        "centos-8",
        "rhel-7",
        "rhel-8",
        "fedora-33",
    ];
    let mut os_rename = HashMap::new();
    os_rename.insert("rhel-7", "redhat-7");
    os_rename.insert("rhel-8", "redhat-8");

    let mut os_attributes = HashMap::new();

    let os_release = std::fs::read_to_string("/etc/os-release").unwrap();
    os_release.lines()
        .map(|line| line.split("=").collect())
        .filter(|parts: &Vec<_>| parts.len() == 2)
        .for_each(|key_value| {
            &os_attributes.insert(key_value[0].replace("\"", ""), key_value[1].replace("\"", ""));
        });

    match (os_attributes.get("ID"), os_attributes.get("VERSION_ID")) {
        (Some(distribution), Some(version)) => {
            known_os.iter().filter_map(|&os| {
                if os.starts_with(format!("{}-{}", distribution, version).as_str()) {
                    os_rename.get(os)
                        .map(|&s| String::from(s))
                        .or(Some(String::from(os)))
                } else {
                    None
                }
            }).next()
        }
        _ => {
            None
        }
    }
}
