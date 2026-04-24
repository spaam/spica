use regex::Regex;
use serde::Deserialize;
use sqlx::postgres::PgPoolOptions;
use std::fs;
use std::process::ExitCode;

#[derive(Deserialize)]
struct User {
    username: String,
    password: String,
    pghost: String,
    pguser: String,
    pgpasswd: String,
    pgdb: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MonthlySummary {
    year: u32,
    month: u32,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Transaction {
    pub transaction_id: String,
    pub transaction_date: String,
    pub store_id: u32,
    pub store_marketing_name: String,
    pub store_city: String,
    pub transaction_chanel: String,
    pub transaction_value: f64,
    pub total_discount: f64,
    pub discount_value: f64,
}

#[derive(Deserialize, Debug)]
pub struct TransactionResponse {
    pub transactions: Vec<Transaction>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SummaryResponse {
    month_summaries: Vec<MonthlySummary>,
}

#[tokio::main]
async fn main() -> ExitCode {
    let content = fs::read_to_string("user.json").expect("Failed to read file");
    let user: User = serde_json::from_str(&content).expect("Failed to parse JSON");

    let client = reqwest::Client::builder()
        .cookie_store(true)
        .build()
        .unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(
            format!(
                "postgres://{}:{}@{}/{}",
                user.pguser, user.pgpasswd, user.pghost, user.pgdb
            )
            .as_str(),
        )
        .await
        .unwrap();
    client
        .get("https://www.ica.se/logga-in/sso/?returnUrl=https%3A%2F%2Fwww.ica.se%2F")
        .send()
        .await
        .unwrap();
    client
        .get("https://ims.icagruppen.se/authn/authenticate/IcaCustomers")
        .send()
        .await
        .unwrap();

    let params = [("userName", user.username), ("password", user.password)];
    let res = client
        .post("https://ims.icagruppen.se/authn/authenticate/IcaCustomers")
        .form(&params)
        .send()
        .await
        .unwrap();
    if !res.status().is_success() {
        eprintln!("Login failed with status: {}", res.status());
        return ExitCode::FAILURE;
    }
    let data = res.text().await.unwrap();
    let token_regex = Regex::new(r#"name="token" value="([^"]+)"#).unwrap();
    let state_regex = Regex::new(r#"name="state" value="([^"]+)"#).unwrap();
    if !token_regex.is_match(&data) || !state_regex.is_match(&data) {
        eprintln!("Failed to extract token or state from the response.");
        return ExitCode::FAILURE;
    }
    let token = token_regex
        .captures(&data)
        .unwrap()
        .get(1)
        .unwrap()
        .as_str();
    let state = state_regex
        .captures(&data)
        .unwrap()
        .get(1)
        .unwrap()
        .as_str();

    let res = client
        .post("https://ims.icagruppen.se/oauth/v2/authorize")
        .form(&[("token", token), ("state", state)])
        .send()
        .await
        .unwrap();
    if !res.status().is_success() {
        eprintln!("Authorization failed with status: {}", res.status());
        return ExitCode::FAILURE;
    }
    let res = client
        .get("https://www.ica.se/api/cpa/purchases/historical/me/monthsummaries")
        .send()
        .await
        .unwrap();
    if !res.status().is_success() {
        eprintln!(
            "Failed to fetch purchase history with status: {}",
            res.status()
        );
        return ExitCode::FAILURE;
    }

    let mut summary_response: SummaryResponse = res.json::<SummaryResponse>().await.unwrap();
    summary_response.month_summaries.reverse();
    for month in summary_response.month_summaries {
        let res = client
            .get(format!(
                "https://www.ica.se/api/cpa/purchases/historical/me/byyearmonth/{}-{:02}",
                month.year, month.month
            ))
            .send()
            .await
            .unwrap();
        if !res.status().is_success() {
            // when they cant fetch it. it shows 400  / 500 for some weird reason
            continue;
        }
        let mut transaction_response: TransactionResponse =
            res.json::<TransactionResponse>().await.unwrap();
        transaction_response.transactions.reverse();
        for transaction in transaction_response.transactions {
            let _ = sqlx::query("INSERT INTO ica_spending (transaction_id, store, discount, transaction, date) VALUES ($1, $2, $3, $4, $5::date) ON CONFLICT (transaction_id) DO NOTHING")
                .bind(&transaction.transaction_id)
                .bind(&transaction.store_marketing_name)
                .bind(transaction.total_discount)
                .bind(transaction.transaction_value)
                .bind(&transaction.transaction_date)
                .execute(&pool)
                .await
                .unwrap();
        }
    }
    pool.close().await;
    return ExitCode::SUCCESS;
}
