/*
Config file for CDK setup.
*/

const DOMAIN: &str = "tinypy.net";

const BASE_DOMAIN: &str = "https://api.cloudflare.com";

const SUBDOMAINS: &[(&str, &str)] = &[
    ("infra", "github.com/dylan-sutton-chavez/tinypy-infra/")
];

const CDN_SUBDOMAIN: &str = "cdn";