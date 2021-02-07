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

#[derive(Debug, Deserialize)]
struct APISysReqs {
    requirements: Vec<APIRequirement>,
}

#[derive(Debug, Deserialize)]
struct APIRequirement {
    name: String,
    requirements: APIPackageRequirements,
}

#[derive(Debug, Deserialize)]
struct APIPackageRequirements {
    packages: Vec<String>,
    pre_install: Option<Vec<APIPrePost>>,
    install_scripts: Vec<String>,
    post_install: Option<Vec<APIPrePost>>,
}

#[derive(Debug, Deserialize)]
struct APIPrePost {
    command: String,
    script: String,
}

fn main() -> Result<()> {
    let opt: Opt = Opt::from_args();
    let (distribution, release) = detect_os(opt.os_name, opt.os_version)?;
    let rspm_status = server_status(&opt.server)?;
    let repositories = server_repositories(&opt.server)?;

    let repository_name = opt.repository.unwrap_or(rspm_status.cran_repo);
    let repository = repositories
        .iter()
        .filter(|&repo| repo.name == repository_name)
        .take(1)
        .next()
        .ok_or(anyhow!(
            "Specified repository '{}' does not exist on the server",
            repository_name
        ))?;

    match opt.action {
        Action::Package { packages } => {
            let response = server_sysreqs(
                &opt.server,
                &distribution,
                &release,
                repository.id,
                &packages,
            )
            .with_context(|| "failed to do get system requirements")?;

            for req in response.requirements {
                println!("# R package: {}", req.name);
                println!(
                    "## System libraries: {}",
                    req.requirements.packages.join(", ")
                );
                if let Some(pre_install) = req.requirements.pre_install {
                    pre_install.iter().for_each(|p| println!("{}", p.script));
                }
                req.requirements
                    .install_scripts
                    .iter()
                    .for_each(|script| println!("{}", script));
                if let Some(post_install) = req.requirements.post_install {
                    post_install.iter().for_each(|p| println!("{}", p.script));
                }
                println!();
            }
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
                println!("{}/{}/latest", opt.server, repository_name);
            } else if binary_repository {
                let distro = rspm_status
                    .distros
                    .iter()
                    .filter(|distro| {
                        distro.distribution == distribution && release.starts_with(&distro.release)
                    })
                    .take(1)
                    .next()
                    .ok_or(anyhow!(
                        "server does not support OS {}-{}",
                        distribution,
                        release,
                    ))?;

                if !rspm_status.binaries_enabled {
                    bail!("binary repositories not enabled on server")
                } else if !distro.binaries {
                    bail!(
                        "binary repositories not enabled for {}-{}",
                        distribution,
                        release
                    )
                } else {
                    println!(
                        "{}/{}/__linux__/{}/latest",
                        opt.server, repository_name, distro.binary_url
                    );
                }
            }
        }
    }

    Ok(())
}

fn server_sysreqs(
    server: &String,
    distribution: &String,
    release: &String,
    repo_id: u64,
    packages: &Vec<String>,
) -> Result<APISysReqs> {
    let mut u = url::Url::parse_with_params(
        format!("{}/__api__/repos/{}/sysreqs", server, repo_id).as_str(),
        &[("distribution", distribution), ("release", release)],
    )
    .with_context(|| "failed to construct server URL")?;
    for pkgname in packages {
        u.query_pairs_mut().append_pair("pkgname", pkgname);
    }

    let http_response = minreq::get(u.as_str())
        .with_timeout(60)
        .send()
        .with_context(|| format!("failed to reach server {}", server))?;
    if http_response.status_code < 200 || http_response.status_code > 299 {
        bail!(format!(
            "failed to reach {}/__api__/repos/{}/sysreqs",
            server, repo_id
        ));
    }

    let api_response = http_response.json().with_context(|| {
        format!(
            "failed to parse JSON response from {}/__api__/repos/{}/sysreqs",
            server, repo_id
        )
    })?;

    Ok(api_response)
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

    let mut os_rename = HashMap::new();
    os_rename.insert("rhel", "redhat");

    let mut os_attributes = HashMap::new();

    let os_release = std::fs::read_to_string("/etc/os-release")
        .with_context(|| format!("failed to read /etc/os-release"))?;

    // read key value pairs from file
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

    match (os_attributes.get("ID"), os_attributes.get("VERSION_ID")) {
        (Some(name), Some(version)) => Ok((String::from(name), String::from(version))),
        _ => bail!("failed to detect linux distribution and/or version"),
    }
}
