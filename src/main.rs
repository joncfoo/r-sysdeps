use std::collections::HashMap;

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
#[structopt(name = "action")]
enum Action {
    /// Get system dependencies for R packages
    #[structopt(name = "package")]
    Package {
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

    /// Repository name (case-sensitive, default value: specified by server)
    #[structopt(short, long)]
    repository: Option<String>,

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

#[derive(Debug, Deserialize)]
struct APIRepository {
    id: u64,
    name: String,
    description: Option<String>,
    #[serde(rename = "type")]
    language: String,
}

fn main() -> Result<()> {
    let mut opt: Opt = Opt::from_args();
    let (distribution, release) = detect_os(opt.os_name, opt.os_version)?;
    let rspm_status = server_status(&opt.server)?;
    let repositories = server_repositories(&opt.server)?;

    if let Some(ref repository) = opt.repository {
        // validate input
        let mut found = false;
        for repo in repositories.iter() {
            if &repo.name == repository {
                found = true;
                break;
            }
        }
        if !found {
            bail!(
                "Specified repository '{}' does not exist on the server",
                repository
            )
        }
    } else {
        opt.repository = Some(rspm_status.cran_repo);
    }

    match opt.action {
        Action::Package { packages } => {
            // TODO
        }
        Action::Repository {
            list,
            binary_repository,
            source_repository,
        } => {
            if list {
                for repo in repositories.iter() {
                    println!("{}", repo.name);
                }
            } else if source_repository {
                println!("{}/{}/latest", opt.server, opt.repository.unwrap());
            } else if binary_repository {
                let distro = rspm_status
                    .distros
                    .iter()
                    .filter(|distro| {
                        distro.distribution == distribution && distro.release == release
                    })
                    .take(1)
                    .next()
                    .ok_or(anyhow!(
                        "OS not supported by server: {}-{}",
                        distribution,
                        release
                    ))?;

                if !rspm_status.binaries_enabled {
                    bail!("binary repositories not enabled on server")
                } else if !distro.binaries {
                    bail!("binary repositories not enabled for specified OS")
                } else {
                    println!(
                        "{}/{}/__linux__/{}/latest",
                        opt.server,
                        opt.repository.unwrap(),
                        distro.binary_url
                    );
                }
            }
        }
    }

    Ok(())
}

fn server_repositories(server: &String) -> Result<Vec<APIRepository>> {
    let http_response = minreq::get(format!("{}/__api__/repos", server))
        .with_timeout(10)
        .send()
        .with_context(|| format!("failed to reach server {}", server))?;

    if http_response.status_code < 200 || http_response.status_code > 299 {
        bail!(format!("failed to reach {}/__api__/repos", server));
    }

    let api_response = http_response.json().with_context(|| {
        format!(
            "failed to parse JSON response from {}/__api__/repos",
            server
        )
    })?;

    Ok(api_response)
}

fn server_status(server: &String) -> Result<APIStatusResponse> {
    let http_response = minreq::get(format!("{}/__api__/status", server))
        .with_timeout(10)
        .send()
        .with_context(|| format!("failed to reach server {}", server))?;

    if http_response.status_code < 200 || http_response.status_code > 299 {
        bail!(format!("failed to reach {}/__api__/status", server));
    }

    let api_response = http_response.json().with_context(|| {
        format!(
            "failed to parse JSON response from {}/__api__/status",
            server
        )
    })?;

    Ok(api_response)
}

fn detect_os(os_name: Option<String>, os_version: Option<String>) -> Result<(String, String)> {
    if let (Some(name), Some(version)) = (os_name, os_version) {
        // user provided so just use it
        return Ok((name, version));
    }

    let known_os = vec![
        "ubuntu-16.04",
        "ubuntu-18.04",
        "ubuntu-20.04",
        "centos-7",
        "centos-8",
        "rhel-7",
        "rhel-8",
    ];
    let mut os_rename = HashMap::new();
    os_rename.insert("rhel-7", "redhat-7");
    os_rename.insert("rhel-8", "redhat-8");

    let mut os_attributes = HashMap::new();

    let os_release = std::fs::read_to_string("/etc/os-release").unwrap();
    os_release
        .lines()
        .map(|line| line.split("=").collect())
        .filter(|parts: &Vec<_>| parts.len() == 2)
        .for_each(|key_value| {
            &os_attributes.insert(
                key_value[0].replace("\"", ""),
                key_value[1].replace("\"", ""),
            );
        });

    let os = match (os_attributes.get("ID"), os_attributes.get("VERSION_ID")) {
        (Some(distribution), Some(version)) => known_os
            .iter()
            .filter_map(|&os| {
                if os.starts_with(format!("{}-{}", distribution, version).as_str()) {
                    let os = os_rename
                        .get(os)
                        .map(|&s| String::from(s))
                        .or(Some(String::from(os)))
                        .unwrap();
                    let os_parts: Vec<&str> = os.split("-").collect();
                    Some((String::from(os_parts[0]), String::from(os_parts[1])))
                } else {
                    None
                }
            })
            .next(),
        _ => None,
    };

    os.ok_or(anyhow!(
        "Failed to auto-detect this OS from the list contained in this tool"
    ))
}
