use clap::Clap;
use crossterm::event::{read, Event};
use jsonschema::{self, Draft, JSONSchema};
use path_absolutize::Absolutize;
use path_slash::{PathBufExt, PathExt};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{self, Value};
use std::{
    collections::{HashMap, HashSet},
    env,
    fs::File,
    io::Read,
    iter::FromIterator,
    path::PathBuf,
};
use std::{fs, io::Cursor, ops::Deref};

//config types

fn default_package_rule_operation() -> OperationEnum {
    OperationEnum::Include
}
fn default_extract_zip() -> ZipExtraction {
    ZipExtraction::Enabled
}

fn default_packages_local_dir() -> String {
    ".".to_string()
}

#[derive(Serialize, Deserialize, Debug)]
enum OperationEnum {
    #[serde(rename = "include")]
    Include,
    #[serde(rename = "exclude")]
    Exclude,
}

#[derive(Serialize, Deserialize, Debug)]
struct PackageSingle {
    id: String,
    #[serde(default = "default_package_rule_operation")]
    operation: OperationEnum,
}
#[derive(Serialize, Deserialize, Debug)]
struct PackageRegex {
    #[serde(default = "default_package_rule_operation")]
    operation: OperationEnum,
    pattern: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
enum PackageRuleEnum {
    #[serde(rename = "regex")]
    Regex(PackageRegex),
    #[serde(rename = "single")]
    Single(PackageSingle),
}

#[derive(Serialize, Deserialize, Debug)]
enum ZipExtraction {
    #[serde(rename = "disabled")]
    Disabled,
    #[serde(rename = "enabled")]
    Enabled,
}

#[derive(Serialize, Deserialize, Debug)]
struct Packages {
    #[serde(default = "default_extract_zip")]
    zip_extraction: ZipExtraction,
    #[serde(default = "default_packages_local_dir")]
    local_dir: String,
    filter_rules: Vec<PackageRuleEnum>,
}

#[derive(Serialize, Deserialize, Debug)]
struct CredentialSUser {
    username: String,
    password_environment_variable: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct CredentialOauthClientCredentials {
    client_id: String,
    token_endpoint_url: String,
    client_secret_environment_variable: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
enum CredentialInside {
    #[serde(rename = "oauth_client_credentials")]
    OauthClientCredentials(CredentialOauthClientCredentials),
    #[serde(rename = "s_user")]
    SUser(CredentialSUser),
}

#[derive(Serialize, Deserialize, Debug)]
struct Tenant {
    management_host: String,
    credential: CredentialInside,
    // credential: CredentialInside,
}

#[derive(Serialize, Deserialize, Debug)]
struct Config {
    cpisync: String,
    tenant: Tenant,
    packages: Packages,
}

//cli type
#[derive(Clap, Debug)]
#[clap(version = "0.2.0", author = "Fatih Pense @ pizug.com")]
struct Opts {
    #[clap(short, long, default_value = "./cpi-sync.json")]
    config: String,
    #[clap(long, about = "Disable features that require user input")]
    no_input: bool,
}

// response types
#[derive(Serialize, Deserialize, Debug)]
struct APIResponseResult {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Mode")]
    mode: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct APIResponseD {
    results: Vec<APIResponseResult>,
}
#[derive(Serialize, Deserialize, Debug)]
struct APIResponseRoot {
    d: APIResponseD,
}

// response types: token api

#[derive(Serialize, Deserialize, Debug)]
struct TokenAPIResponseRoot {
    access_token: String,
}

fn pause() {
    println!("Press any key to continue...");
    loop {
        // `read()` blocks until an `Event` is available
        match read().unwrap() {
            Event::Key(_) => {
                // println!("{:?}", event);
                break;
            }
            _ => {}
        }
    }
}

async fn process_package(
    package_id: &str,
    config: &Config,
    client: &reqwest::Client,
    authorization: &str,
    data_dir: &std::path::PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Processing Package: {:?}", package_id);
    let api_package_artifact_list_url = format!(
        "https://{host}/api/v1/IntegrationPackages('{package_id}')/IntegrationDesigntimeArtifacts",
        host = config.tenant.management_host,
        package_id = package_id
    );
    let resp = client
        .get(&api_package_artifact_list_url)
        .header("Authorization", authorization)
        .header("Accept", "application/json")
        .send()
        .await?;

    let resp_success = &resp.status().is_success();
    let resp_code = resp.status();

    let body_text = resp.text().await?;

    if !resp_success {
        println!("API Package List Artifacts Failed!");
        println!("API URL: {}", &api_package_artifact_list_url);
        println!("API Response Code: {:#?}", &resp_code);
        println!("Response Body:");
        println!("{}", &body_text);
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "API Package List Artifacts Failed!",
        )
        .into());
    }

    let resp_obj: APIResponseRoot = match serde_json::from_slice(body_text.as_bytes()) {
        Ok(api_resp) => api_resp,
        Err(err) => {
            println!("Package Download Failed!");
            println!("API URL: {}", &api_package_artifact_list_url);
            println!("API Response Code: {:#?}", &resp_code);
            println!("Response Body:");
            println!("{}", &body_text);
            return Err(std::io::Error::new(std::io::ErrorKind::Other, err).into());
        }
    };

    for artifact in resp_obj.d.results {
        println!("- Artifact: {:#?}", artifact.id);

        let api_artifact_payload_url = format!("https://{host}/api/v1/IntegrationDesigntimeArtifacts(Id='{artifact_id}',Version='Active')/$value",
        host=config.tenant.management_host,artifact_id= artifact.id);
        let resp = client
            .get(&api_artifact_payload_url)
            .header("Authorization", authorization)
            .send()
            .await?;

        let resp_success = &resp.status().is_success();
        let resp_code = resp.status();

        if !resp_success {
            println!("Artifact Download Failed!");
            println!("API URL: {}", &api_artifact_payload_url);
            println!("API Response Code: {:#?}", &resp_code);
            println!("Response Body:");
            let body_text = resp.text().await?;
            println!("{}", &body_text);
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "API Artifact Download Failed!",
            )
            .into());
        }

        let respbytes = resp.bytes().await?;
        let mut respbytes_cursor = Cursor::new(respbytes.deref());

        match config.packages.zip_extraction {
            ZipExtraction::Disabled => {
                let write_dir = data_dir
                    .join(&package_id)
                    .join(artifact.id.to_string() + ".zip");

                let parent_dir = write_dir.parent().unwrap();
                fs::create_dir_all(parent_dir).unwrap();

                let mut write_dir = fs::File::create(&write_dir).unwrap();
                std::io::copy(&mut respbytes_cursor, &mut write_dir).unwrap();
            }
            ZipExtraction::Enabled => {
                let mut archive = zip::ZipArchive::new(respbytes_cursor).unwrap();

                for i in 0..archive.len() {
                    let mut file = archive.by_index(i).unwrap();

                    let outpath_str = file.enclosed_name().unwrap().to_str().unwrap();
                    let outpath: PathBuf = PathBuf::from_slash(outpath_str);

                    // println!(
                    //     "data_dir: {:?} , package_id:{:?} , artifact_id: {:?}, outpath: {:?}",
                    //     &data_dir, &package_id, &artifact.id, &outpath
                    // );
                    let write_dir = data_dir
                        .canonicalize()
                        .unwrap()
                        .join(&package_id)
                        .join(&artifact.id)
                        .join(outpath);
                    // println!("write_dir: {:?} ", &write_dir);

                    let parent_dir = write_dir.parent().unwrap();
                    fs::create_dir_all(parent_dir).unwrap();

                    let mut write_dir = fs::File::create(&write_dir).unwrap();
                    std::io::copy(&mut file, &mut write_dir).unwrap();
                }
            }
        }
    }
    Ok(())
}

async fn get_all_packages(
    config: &Config,
    client: &reqwest::Client,
    authorization: &str,
) -> Result<APIResponseRoot, Box<dyn std::error::Error>> {
    let api_package_list_url = format!(
        "https://{host}/api/v1/IntegrationPackages",
        host = config.tenant.management_host
    );
    let resp = client
        .get(&api_package_list_url)
        .header("Authorization", authorization)
        .header("Accept", "application/json")
        .send()
        .await?;

    let resp_success = &resp.status().is_success();
    let resp_code = resp.status();

    let body_text = resp.text().await?;

    if !resp_success {
        println!("Package List Failed!");
        println!("API URL: {}", &api_package_list_url);
        println!("API Response Code: {:#?}", &resp_code);
        println!("Response Body:");
        println!("{}", &body_text);
        return Err(
            std::io::Error::new(std::io::ErrorKind::Other, "API Package List  Failed!").into(),
        );
    }

    let resp_obj: APIResponseRoot = match serde_json::from_slice(body_text.as_bytes()) {
        Ok(api_resp) => api_resp,
        Err(err) => {
            println!("Package List Failed!");
            println!("API URL: {}", &api_package_list_url);
            println!("API Response Code: {:#?}", &resp_code);
            println!("Response Body:");
            println!("{}", &body_text);
            return Err(std::io::Error::new(std::io::ErrorKind::Other, err).into());
        }
    };
    //println!("{:?}", &resp_obj);

    Ok(resp_obj)
}

async fn run(opts: &Opts) -> Result<(), Box<dyn std::error::Error>> {
    println!("Start CPI Sync?");
    if !opts.no_input {
        pause();
    }

    let schema_str = include_str!("../resources/config.schema.json");
    let json_schema: Value = serde_json::from_str(schema_str).unwrap();

    let compiled_schema = JSONSchema::options()
        .with_draft(Draft::Draft7)
        .compile(&json_schema)?;

    let mut config_str = String::new();
    File::open(&opts.config)?.read_to_string(&mut config_str)?;
    // let reader = BufReader::new(file);

    // Read the JSON contents of the file as an instance of `User`.
    let config_json: serde_json::Value = serde_json::from_str(&config_str)?;

    let result = compiled_schema.validate(&config_json);
    if let Err(errors) = result {
        for error in errors {
            println!("Validation error: {}", error);
        }
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "JSON Schema validation error.",
        )
        .into());
    }

    let config: Config = serde_json::from_str(&config_str)?;

    //println!("config: {:?}", config);
    //println!("Using input file: {:?}", opts);

    let client = reqwest::Client::new();

    // let mut authorization: Option<String> = None;

    let mut password: Option<String> = None;

    //get secret from environment variable
    match &config.tenant.credential {
        CredentialInside::SUser(c) => {
            match &c.password_environment_variable {
                Some(varkey) => {
                    match env::var(varkey) {
                        Ok(val) => {
                            password = Some(val);
                        }
                        Err(e) => {
                            println!(
                                "Can not find S-user Pass in environment variable: {}: {}",
                                &varkey, e
                            );
                            // return Err(e.into());
                        }
                    };
                }
                None => (),
            };
        }
        CredentialInside::OauthClientCredentials(c) => {
            match &c.client_secret_environment_variable {
                Some(varkey) => {
                    match env::var(varkey) {
                        Ok(val) => {
                            password = Some(val);
                        }
                        Err(e) => {
                            println!(
                                "Can not find Client Secret environment variable: {}: {}",
                                &varkey, e
                            );
                        }
                    };
                }
                None => (),
            };
        }
    }

    let username: String = match &config.tenant.credential {
        CredentialInside::OauthClientCredentials(c) => c.client_id.to_string(),
        CredentialInside::SUser(c) => c.username.to_string(),
    };
    //try to get password from command line
    if !opts.no_input {
        match &password {
            None => {
                let message = format!(
                    "Would you like to enter a password for user: {user} to connect host: {host}?",
                    user = username,
                    host = config.tenant.management_host
                );

                println!("{}", message);

                let pass = rpassword::prompt_password_stdout("Password: ")?;
                password = Some(pass);
                //println!("Your password is {}", pass);
            }
            _ => {}
        }
    }

    let password: String = match password {
        Some(p) => p,
        None => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Could not use any password/secret",
            )
            .into())
        }
    };

    let check_api_url = format!(
        "https://{host}/api/v1/",
        host = &config.tenant.management_host
    );

    //for oauth we need to get the token
    let authorization = match &config.tenant.credential {
        CredentialInside::OauthClientCredentials(c) => {
            let api_token_url = format!(
                "{url}?grant_type=client_credentials",
                url = c.token_endpoint_url
            );
            let auth = basic_auth(&c.client_id, &password);

            let resp = client
                .post(&api_token_url)
                .header("Authorization", auth)
                .send()
                .await?;
            println!("Token API status: {:?}", resp.status());
            let respbody = resp.json::<TokenAPIResponseRoot>().await?;

            format!("Bearer {token}", token = respbody.access_token)
        }
        CredentialInside::SUser(c) => basic_auth(&c.username, &password),
    };

    let resp = client
        .get(&check_api_url)
        .header("Authorization", &authorization)
        .send()
        .await?;

    let resp_success = &resp.status().is_success();
    let resp_code = resp.status();

    if !resp_success {
        println!("API First Check Failed!");
        println!("API Response Code: {:#?}", resp_code);
        return Err(std::io::Error::new(std::io::ErrorKind::Other, "API Check Failed!").into());
    } else {
        println!("API First Check Successful.");
    }

    let mut data_dir = std::path::PathBuf::from(&opts.config);
    data_dir = data_dir
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap()
        .join(&config.packages.local_dir)
        .to_path_buf();

    let api_package_list = get_all_packages(&config, &client, &authorization).await?;

    let mut api_package_set: HashSet<String> = HashSet::new();
    let mut api_package_name_map: HashMap<String, String> = HashMap::new();
    for package in api_package_list.d.results.iter() {
        api_package_set.insert(package.id.to_string());
        match api_package_name_map.entry(package.name.to_string()) {
            std::collections::hash_map::Entry::Occupied(mut e) => {
                e.insert(e.get().clone() + "," + &package.id);
            }
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(package.id.to_string());
            }
        };
    }

    let mut operating_package_set: HashSet<String> = HashSet::new();

    for package_rule in config.packages.filter_rules.iter() {
        let mut rule_package_set: HashSet<String> = HashSet::new();
        match package_rule {
            PackageRuleEnum::Regex(rule) => {
                let re = Regex::new(&rule.pattern)?;

                for p in &api_package_set {
                    if re.is_match(&p) {
                        rule_package_set.insert(p.clone());
                    }
                }

                // rule.operation
                match rule.operation {
                    OperationEnum::Include => {
                        //operating_package_set.union(&rule_package_set);
                        operating_package_set.extend(rule_package_set);
                    }
                    OperationEnum::Exclude => {
                        operating_package_set = operating_package_set
                            .difference(&rule_package_set)
                            .cloned()
                            .collect();
                    }
                }
            }
            PackageRuleEnum::Single(rule) => {
                //if single package rule not found in original package list check names and inform.
                if !api_package_set.contains(&rule.id) {
                    println!("Package ID not found: {}", &rule.id);

                    match api_package_name_map.get(&rule.id) {
                        Some(id_for_name) => {
                            println!(
                                "Did you enter the Package name instead of this Package ID?: '{}'",
                                id_for_name
                            );
                        }
                        None => {}
                    }

                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "Package ID not found!",
                    )
                    .into());
                }

                match rule.operation {
                    OperationEnum::Include => {
                        operating_package_set.insert(rule.id.clone());
                    }
                    OperationEnum::Exclude => {
                        operating_package_set.remove(&rule.id);
                    }
                }
            }
        }
    }

    let package_list: Vec<String> = Vec::from_iter(operating_package_set);

    println!("Downloading These Packages:");
    println!("{:?}", &package_list);

    //fetch package artifacts
    for package_id in package_list.iter() {
        process_package(package_id, &config, &client, &authorization, &data_dir).await?;
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let opts: Opts = Opts::parse();
    let result = run(&opts).await;

    match result {
        Ok(()) => {
            println!("Completed successfully.");
            if !opts.no_input {
                pause();
            }
            return Ok(());
        }
        Err(err) => {
            println!("{:?}", err);
            if !opts.no_input {
                pause();
            }
            return Err(err.into());
        }
    };
}

fn basic_auth(user: &str, pass: &str) -> String {
    let encoded = base64::encode(format!("{username}:{pass}", username = &user, pass = &pass));
    let authorization = format!("Basic {encoded}", encoded = encoded);
    return authorization;
}
