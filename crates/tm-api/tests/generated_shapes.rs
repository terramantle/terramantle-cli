//! Shape tests for the generated models: the cases where a plausible-looking
//! generator output is still wrong.
//!
//! The deserialisation policy here is deliberately permissive — response fields
//! are `Option<T>` and nothing denies unknown fields — so most shape mistakes do
//! not throw, they silently produce `None`. These tests target the places where
//! that permissiveness could hide something.

use tm_api::generated::models::*;

/// The one genuine `oneOf` in the spec, and the reason `required` is kept on
/// union arms.
///
/// typify renders the union `#[serde(untagged)]`, so serde takes the FIRST arm
/// that deserializes. `ModulePublishPending`'s four required fields are what
/// distinguish it; without them it matches any object and a settled publish
/// would be reported as pending with every field `None`.
#[test]
fn module_publish_union_tells_pending_from_settled() {
    let pending = r#"{
        "ok": true,
        "status": "pending",
        "storage_key": "modules/acme/vpc/aws/1.0.0.tar.gz",
        "version_id": "11111111-1111-1111-1111-111111111111"
    }"#;
    match serde_json::from_str::<PublishModuleVersionResponse>(pending).expect("pending parses") {
        PublishModuleVersionResponse::Pending(p) => {
            // These four are non-Option precisely because they discriminate.
            assert_eq!(p.status, "pending");
            assert!(p.ok);
            assert!(p.storage_key.ends_with(".tar.gz"));
        }
        PublishModuleVersionResponse::Settled(_) => panic!("a pending publish read as settled"),
    }

    // A settled body carries `ok`, `status` and `version_id` too, but NOT
    // `storage_key` — which is what keeps it out of the Pending arm. (Pending
    // reports `missing field storage_key`, so serde moves to the next variant.)
    let settled = r#"{
        "ok": true,
        "status": "published",
        "version_id": "22222222-2222-2222-2222-222222222222",
        "version": "1.0.0",
        "consumable": true,
        "deprecated": false,
        "namespace": "acme",
        "name": "vpc"
    }"#;
    match serde_json::from_str::<PublishModuleVersionResponse>(settled).expect("settled parses") {
        PublishModuleVersionResponse::Settled(s) => {
            assert_eq!(s.consumable, Some(true));
            assert_eq!(s.version.as_deref(), Some("1.0.0"));
            // `status` is the ModuleScanStatus enum here, not free text: the
            // generated type rejected "complete", which is not one of the four
            // the spec declares. That is the enum earning its keep.
            assert_eq!(s.status, Some(ModuleScanStatus::Published));
        }
        PublishModuleVersionResponse::Pending(_) => panic!(
            "a settled publish read as PENDING — the union's discriminating `required` fields \
             were stripped, so the first arm swallowed the second"
        ),
    }
}

/// The envelopes the CLI unwraps. A missing key must read as empty rather than
/// failing: an org with no providers yet answers `{}` in some paths.
#[test]
fn list_envelopes_tolerate_a_missing_key() {
    assert!(serde_json::from_str::<ListOrgsResponse>("{}")
        .unwrap()
        .orgs
        .is_empty());
    assert!(serde_json::from_str::<ListOrgProvidersResponse>("{}")
        .unwrap()
        .providers
        .is_empty());
    assert!(serde_json::from_str::<ListStateWorkspacesResponse>("{}")
        .unwrap()
        .workspaces
        .is_empty());
}

/// Unknown fields must never fail a parse: the server adds columns, and an older
/// CLI has to keep working. This is the rubric the normaliser preserves.
#[test]
fn unknown_fields_are_ignored_everywhere_that_matters() {
    let body = r#"{"orgs":[{
        "id":"t1","name":"Acme","slug":"acme","is_personal":0,"tier":"team",
        "role":"owner","created_at":1,"claimed_domain":null,"is_demo":0,
        "a_column_added_next_week": {"nested": true}
    }]}"#;
    let resp: ListOrgsResponse = serde_json::from_str(body).expect("extra columns tolerated");
    assert_eq!(resp.orgs[0].slug, "acme");
    assert_eq!(resp.orgs[0].role, Role::Owner);
}

/// `null` and an absent key must both read as "no lock", and must stay
/// distinguishable from a lock that exists.
#[test]
fn state_lock_envelope_distinguishes_absent_from_held() {
    assert!(serde_json::from_str::<GetStateLockResponse>("{}")
        .unwrap()
        .lock
        .is_none());
    assert!(
        serde_json::from_str::<GetStateLockResponse>(r#"{"lock":null}"#)
            .unwrap()
            .lock
            .is_none()
    );
    let held: GetStateLockResponse =
        serde_json::from_str(r#"{"lock":{"lock_id":"abc","who":"rhys@example.com"}}"#).unwrap();
    let lock = held.lock.expect("a held lock");
    assert_eq!(lock.lock_id.as_deref(), Some("abc"));
    assert_eq!(lock.who.as_deref(), Some("rhys@example.com"));
}

/// The severity counts `providers ls --at-risk` filters on must survive the
/// round trip. These are the nine fields the spec previously failed to declare.
#[test]
fn provider_summary_carries_the_severity_counts() {
    let body = r#"{"providers":[{
        "id":"p1","namespace":"hashicorp","type":"aws","upstream":"terraform.io",
        "serve_mode":"serve","version_constraint":"~> 5.0",
        "vuln_count":9,"critical_count":2,"high_count":3,
        "version_count":12,"latest_version":"5.4.1","outdated":true,
        "usage_workspace_count":4
    }]}"#;
    let resp: ListOrgProvidersResponse = serde_json::from_str(body).unwrap();
    let p = &resp.providers[0];
    assert_eq!(p.critical_count, Some(2));
    assert_eq!(p.high_count, Some(3));
    assert_eq!(p.vuln_count, Some(9));
    assert_eq!(p.outdated, Some(true));
    // Identity stays non-optional, so call sites need no unwrap.
    assert_eq!(p.namespace, "hashicorp");
    assert_eq!(p.type_, "aws");
}

/// The nullable enum the normaliser collapsed. `None` is the `null` the schema
/// describes — for a serve override, "clear it".
#[test]
fn serve_override_is_a_plain_enum_with_none_as_null() {
    let cleared: Option<ServeOverride> = serde_json::from_str("null").unwrap();
    assert!(cleared.is_none());
    let blocked: ServeOverride = serde_json::from_str(r#""block""#).unwrap();
    assert_eq!(blocked, ServeOverride::Block);
    assert_eq!(
        serde_json::to_string(&ServeOverride::Allow).unwrap(),
        r#""allow""#
    );
}
