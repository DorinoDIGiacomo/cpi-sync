use clap::Clap;
use crossterm::event::{read, Event};
use jsonschema::{self, Draft, JSONSchema};
use serde::{Deserialize, Serialize};
use serde_json::{self, Value};
use std::{env, fs::File, io::Read};
use std::{fs, io::Cursor, ops::Deref};

//config types
#[derive(Serialize, Deserialize, Debug)]
struct Package {
    id: String,
    local_dir: Option<String>,
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
    packages: Vec<Package>,
}

//cli type
#[derive(Clap, Debug)]
#[clap(version = "0.1.0", author = "Fatih Pense @ pizug.com")]
struct Opts {
    #[clap(short, long, default_value = "./cpi-sync.json")]
    config: String,
    #[clap(long)]
    no_input: bool,
}

// response types
#[derive(Serialize, Deserialize, Debug)]
struct APIResponseResult {
    #[serde(rename = "Id")]
    id: String,
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
    }

    let config: Config = serde_json::from_str(&config_str)?;

    // println!("config: {:?}", config);
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
        println!("API Check Failed!");
        println!("API Response Code: {:#?}", resp_code);
        return Err(std::io::Error::new(std::io::ErrorKind::Other, "API Check Failed!").into());
    }

    let mut data_dir = std::path::PathBuf::from(&opts.config);
    data_dir = data_dir.parent().unwrap().to_path_buf();

    //fetch package artifacts
    for package in config.packages.iter() {
        let package_path = match &package.local_dir {
            Some(str) => &str,
            None => &package.id,
        };

        println!("Processing Package: {:?}", package);
        let api_package_artifact_list_url= format!("https://{host}/api/v1/IntegrationPackages('{package_id}')/IntegrationDesigntimeArtifacts",
        host=config.tenant.management_host,package_id= package.id);
        let resp = client
            .get(&api_package_artifact_list_url)
            .header("Authorization", &authorization)
            .header("Accept", "application/json")
            .send()
            .await?
            .json::<APIResponseRoot>()
            .await?;

        for artifact in resp.d.results {
            println!("- Artifact: {:#?}", artifact.id);

            let api_artifact_payload_url = format!("https://{host}/api/v1/IntegrationDesigntimeArtifacts(Id='{artifact_id}',Version='Active')/$value",
            host=config.tenant.management_host,artifact_id= artifact.id);
            let resp = client
                .get(&api_artifact_payload_url)
                .header("Authorization", &authorization)
                .send()
                .await?;

            let respbytes = resp.bytes().await?;
            let respbytes_cursor = Cursor::new(respbytes.deref());

            let mut archive = zip::ZipArchive::new(respbytes_cursor).unwrap();

            for i in 0..archive.len() {
                let mut file = archive.by_index(i).unwrap();

                let outpath = file.enclosed_name().unwrap().to_owned();

                let write_dir = data_dir
                    .join(&package_path)
                    .join(&artifact.id)
                    .join(outpath);

                fs::create_dir_all(&write_dir.parent().unwrap()).unwrap();

                let mut write_dir = fs::File::create(&write_dir).unwrap();
                std::io::copy(&mut file, &mut write_dir).unwrap();
            }
        }
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
