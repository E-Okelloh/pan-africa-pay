//! Live verification against the real Daraja sandbox.
//!
//! Reads `MPESA_*` variables (loaded from `.env` via dotenvy), fires an
//! STK push to the sandbox test phone, then polls with `stk_query` so
//! we can confirm the prompt actually landed.
//!
//! Run:
//! ```sh
//! cargo run -p pan-africa-pay-mpesa --example stk_push
//! ```

use std::time::Duration;

use pan_africa_pay_mpesa::config::{Environment, MpesaConfig};
use pan_africa_pay_mpesa::types::StkPushRequest;
use pan_africa_pay_mpesa::MpesaClient;

const TEST_PHONE: &str = "254708374149";

fn env_var(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        eprintln!("missing required env var {name} (set it in .env)");
        std::process::exit(1);
    })
}

fn config() -> MpesaConfig {
    MpesaConfig {
        consumer_key: env_var("MPESA_CONSUMER_KEY"),
        consumer_secret: env_var("MPESA_CONSUMER_SECRET"),
        passkey: env_var("MPESA_PASSKEY"),
        short_code: env_var("MPESA_SHORT_CODE"),
        callback_url: env_var("MPESA_CALLBACK_URL"),
        environment: Environment::Sandbox,
        timeout_secs: 30,
        token_ttl_secs: 3_500,
        base_url_override: String::new(),
    }
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let config = config();
    let client = MpesaClient::from_config(config).expect("valid config");

    let amount = std::env::var("MPESA_AMOUNT").unwrap_or_else(|_| "1".to_string());

    let request = StkPushRequest {
        business_short_code: "174379".to_string(),
        password: String::new(),
        timestamp: String::new(),
        transaction_type: "CustomerPayBillOnline".to_string(),
        amount,
        party_a: TEST_PHONE.to_string(),
        party_b: "174379".to_string(),
        phone_number: TEST_PHONE.to_string(),
        callback_url: std::env::var("MPESA_CALLBACK_URL").expect("callback url"),
        account_reference: "PANAFRICA-PAY".to_string(),
        transaction_desc: "Verification".to_string(),
    };

    println!("sending STK push to {TEST_PHONE} ...");
    match client.stk_push(&request).await {
        Ok(ack) => {
            println!(
                "initiated: ResponseCode={} CheckoutRequestID={:?}",
                ack.response_code, ack.checkout_request_id
            );
            let checkout_id = match ack.checkout_request_id {
                Some(id) => id,
                None => {
                    eprintln!("no CheckoutRequestID in acknowledgement");
                    std::process::exit(1);
                }
            };
            poll(&client, &checkout_id).await;
        }
        Err(err) => {
            eprintln!("STK push failed: {err}");
            std::process::exit(1);
        }
    }
}

async fn poll(client: &MpesaClient, checkout_id: &str) {
    println!("polling for result (up to 60s) ...");
    for _ in 0..12 {
        tokio::time::sleep(Duration::from_secs(5)).await;
        match client.stk_query(checkout_id).await {
            Ok(status) => {
                let code = status.result_code.as_deref().unwrap_or("?");
                println!("result_code={code} result_desc={:?}", status.result_desc);
                if code == "0" {
                    println!("prompt confirmed delivered");
                    return;
                }
            }
            Err(err) => println!("query error (retrying): {err}"),
        }
    }
    println!("timed out waiting for result");
}
