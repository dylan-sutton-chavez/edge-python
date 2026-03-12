use deployment::cloudflare::Cloudflare;
use mockito::{Server, ServerGuard};

async fn setup() -> (Cloudflare, ServerGuard) {

    /*
    Create a mock server and Cloudflare instance for testing.
    */

    let mut server = Server::new_async().await;

    // Server guard keeps mock server alive until test ends.
    server
        .mock("GET", "/client/v4/zones?name=mock.com")
        .with_status(200)
        .with_body(r#"{"result":[{"id":"zone123"}]}"#)
        .create();

    let cf = Cloudflare::new(
        "fake_token".to_string(),
        "mock.com".to_string(),
        server.url(),
        "cdn".to_string(),
        "fake_account".to_string(),
        "fake_access".to_string(),
        "fake_secret".to_string()
    ).await;

    (cf, server)

}

#[tokio::test]
async fn test_create_cname() -> Result<(), Box<dyn std::error::Error>> {

    /*
    Test mocking Cloudflare API to verify CNAME record creation using fake server.
    */

    let (cf, _server) = setup().await;
    
    assert!(cf.create_cname_records(&[("api", "target.com")]).await.is_ok());
    Ok(())

}

#[tokio::test]
async fn test_upload_file() -> Result<(), Box<dyn std::error::Error>> {

    /*
    Test mocking Cloudflare API to verify file upload returns correct CDN URL.
    */

    let (cf, mut server) = setup().await;

    server
        .mock("POST", "/client/v4/zones/zone123/purge_cache")
        .with_status(200)
        .with_body(r#"{"success":true}"#)
        .create();

    let url = cf.upload_file("images/foto.jpg", b"fake_bytes", "image/jpeg").await?;

    assert_eq!(url, "https://cdn.mock.com/images/foto.jpg");
    Ok(())

}