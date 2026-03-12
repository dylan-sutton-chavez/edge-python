use serde_json::json;
use reqwest::Client;
use log::{info, error};
use cloudflare_r2_rs::r2::{R2Manager, R2Endpoint};

pub struct Cloudflare {

    /*
    Structure that contains the `CF_TOKEN` and cloudflare client.
    */

    token: String,
    client: Client,
    domain: String,
    base: String,
    cdn_subdomain: String,
    r2: R2Manager

}

impl Cloudflare {

    pub async fn new(token: String, domain: String, base: String, cdn_subdomain: String, account_id: String, r2_access: String, r2_secret: String) -> Self {

        /*
        Create a implementation instance using the cloudflare setup structure.
        */

        let r2 = R2Manager::new(
            &cdn_subdomain,
            R2Endpoint::Http(
                format!("https://{account_id}.r2.cloudflarestorage.com")
            ),
            &r2_access,
            &r2_secret,
            None
        ).await;

        r2.create_bucket().await;

        Self { token, domain, base, cdn_subdomain, client: Client::new(), r2 }

    }

    async fn get_json(&self, url: &str) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        
        /*
        Get a JSON using a get method, `CF_TOKEN`, asynchronous and JSON format.
        */

        Ok(self.client.get(url).bearer_auth(&self.token).send().await?.json().await?)

    }

    async fn obtain_zone_id(&self) -> Result<String, Box<dyn std::error::Error>> {

        /*
        Obtain the zone id asigned by cloudflare to the domain.
        */

        let res = self.get_json(&format!("{}/client/v4/zones?name={}", &self.base, &self.domain)).await?;

        Ok(res["result"][0]["id"].as_str().ok_or("Zone not found")?.to_string())

    }

    pub async fn upload_file(&self, key: &str, bytes: &[u8], content_type: &str) -> Result<String, Box<dyn std::error::Error>> {

        /*
        Upload a file on the CDN and clean the cache. 
        */

        self.r2.upload(key, bytes, Some("public, max-age=31536000"), Some(content_type)).await;

        let cdn_subdomain_name = &self.cdn_subdomain;
        let domain_name = &self.domain;
        
        let url = format!("https://{cdn_subdomain_name}.{domain_name}/{key}");

        let zone_id = self.obtain_zone_id().await?;

        self.client.post(&format!("{}/client/v4/zones/{zone_id}/purge_cache", &self.base))
            .bearer_auth(&self.token)
            .json(&json!({ "files": [url.clone()] }))
            .send().await?;

        info!("The file has been uploaded (cache purged): {url}.");

        Ok(url)
        
    }

    pub async fn create_cname_records(&self, subdomains: &[(&str, &str)]) -> Result<(), Box<dyn std::error::Error>> {

        /*
        Create a CNAME proxied for each subdomain on config list.
        */

        let zone_id = self.obtain_zone_id().await?;

        for (sub, target) in subdomains {

            let full_name = format!("{sub}.{}", &self.domain);

            let body = json!({ "type" : "CNAME", "name" : full_name, "content" : target, "ttl" : 120, "proxied" : true });

            let url = format!("{}/client/v4/zones/{zone_id}/dns_records", &self.base);

            let res = self.client.post(&url).bearer_auth(&self.token).json(&body).send().await?;
            let status = res.status();

            if status.is_success() {

                info!("CNAME proxied created; {full_name} -> {target}.");

            } 
            
            else {

                error!("An error has occurred creating {full_name}: status {status}.");
            
            }

        }

        Ok(())
    
    }

}

/*
Docs:

use config::{
    DOMAIN, 
    SUBDOMAINS,
    BASE_DOMAIN, 
    CDN_SUBDOMAIN
};

let cf = Cloudflare::new(
    std::env::var("CF_TOKEN")?,
    DOMAIN.to_string(),
    BASE_DOMAIN.to_string(),
    CDN_SUBDOMAIN.to_string(),
    std::env::var("CF_ACCOUNT_ID")?,
    std::env::var("R2_ACCESS_KEY")?,
    std::env::var("R2_SECRET_KEY")?,
).await;

cf.create_cname_records(SUBDOMAINS).await?;
*/