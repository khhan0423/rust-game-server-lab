//! Real operator API authentication, credential projection, and enforcement.

use super::*;

#[sqlx::test(migrator = "db::login::LOGIN_MIGRATOR")]
async fn operator_api_requires_identity_hides_credentials_and_audits_a_sanction(pool: PgPool) {
    let mut child = publisher("operator", &pool).await.spawn().unwrap();
    let (_, api, ops) = ready_addresses(&mut child).await;
    let client = Client::builder().timeout(WAIT).build().unwrap();
    let accounts = format!("http://{ops}/v1/accounts");
    for key in [None, Some("incorrect-operator-key")] {
        let request = client.get(&accounts).header("x-operator", "Support");
        let request = match key {
            Some(key) => request.header("x-ops-key", key),
            None => request,
        };
        assert_eq!(
            request.send().await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        client
            .get(&accounts)
            .header("x-ops-key", OPS_KEY)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );

    let guest: Value = client
        .post(format!("http://{api}/v1/guest/accounts"))
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = guest["guest_id"].as_str().unwrap();
    let secret = guest["secret"].as_str().unwrap();
    let login = || {
        client
            .post(format!("http://{api}/v1/guest/login"))
            .json(&json!({"guest_id": id, "secret": secret, "game_id": "lab"}))
    };
    let entry: Value = login()
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let ticket = entry["ticket"].as_str().unwrap();
    let authorized = |method, url: &str| {
        client
            .request(method, url)
            .header("x-ops-key", OPS_KEY)
            .header("x-operator", "Support")
    };
    let listing: Value = authorized(reqwest::Method::GET, &format!("{accounts}?search={id}"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let row = &listing["accounts"][0];
    let account_id = row["account_id"].as_str().unwrap();
    assert_eq!(row.as_object().unwrap().len(), 3);
    let detail_url = format!("{accounts}/{account_id}");
    let detail: Value = authorized(reqwest::Method::GET, &detail_url)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(detail["account"], *row);
    assert_eq!(
        detail["identities"],
        json!([{"provider": "guest", "subject": id}])
    );
    assert_eq!(detail["tickets"][0].as_object().unwrap().len(), 3);
    assert_eq!(detail["tickets"][0]["game_id"], "lab");
    assert_eq!(detail["tickets"][0]["consumed"], false);
    assert!(detail["tickets"][0]["expires_at"].is_string());
    for payload in [listing.to_string(), detail.to_string()] {
        for forbidden in [secret, ticket, "secret", "hash"] {
            assert!(!payload.contains(forbidden));
        }
    }

    let reason = "Test account violated policy";
    let result: Value = authorized(reqwest::Method::POST, &format!("{detail_url}/status"))
        .json(&json!({"status": "sanctioned", "reason": reason}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(result["changed"], true);
    assert_error(
        login().send().await.unwrap(),
        StatusCode::FORBIDDEN,
        "account_blocked",
    )
    .await;
    let audit: Value = authorized(reqwest::Method::GET, &format!("http://{ops}/v1/audit"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let record = &audit["records"][0];
    assert_eq!(record["account_id"], account_id);
    assert_eq!(record["operator"], "Support");
    assert_eq!(record["reason"], reason);
    assert_eq!(record["before"], "active");
    assert_eq!(record["after"], "sanctioned");
}
