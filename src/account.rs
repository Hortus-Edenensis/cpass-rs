use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use reqwest::Url;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(test)]
use serde_json::json;

use crate::transport::{Session, encrypt_login, inf_enc_sign, timestamp};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Account {
    pub puid: u64,
    pub name: String,
    pub phone: String,
    pub school: String,
    pub stu_id: Option<String>,
    pub sex: i64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct QrLogin {
    pub uuid: String,
    pub enc: String,
    pub url: String,
}

fn form(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn number(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}

pub fn account(session: &Session) -> Result<Account> {
    let value = session
        .get(
            "https://sso.chaoxing.com/apis/login/userLogin4Uname.do",
            &[],
        )?
        .json()?;
    ensure!(
        number(&value["result"]).is_some_and(|result| result != 0),
        "action-required: session expired; log in again"
    );
    let msg = value
        .get("msg")
        .filter(|v| v.is_object())
        .context("invalid account response")?;
    Ok(Account {
        puid: number(&msg["puid"]).context("account response is missing UID")?,
        name: msg["name"].as_str().unwrap_or_default().to_owned(),
        phone: msg["phone"].as_str().unwrap_or_default().to_owned(),
        school: msg["schoolname"].as_str().unwrap_or_default().to_owned(),
        stu_id: msg["uname"].as_str().map(str::to_owned),
        sex: msg["sex"]
            .as_i64()
            .or_else(|| msg["sex"].as_str()?.parse().ok())
            .unwrap_or(-1),
    })
}

pub fn login_password(session: &Session, phone: &str, password: &str) -> Result<Account> {
    ensure!(
        !phone.trim().is_empty() && !password.is_empty(),
        "phone and password are required"
    );
    let encrypted_phone = encrypt_login(phone)?;
    let encrypted_password = encrypt_login(password)?;
    let value = session
        .post_form(
            "https://passport2.chaoxing.com/fanyalogin",
            &[],
            &form(&[
                ("fid", "-1"),
                ("uname", &encrypted_phone),
                ("password", &encrypted_password),
                ("t", "true"),
                ("forbidotherlogin", "0"),
                ("validate", ""),
            ]),
        )?
        .json()?;
    ensure!(
        value["status"].as_bool() == Some(true),
        "login rejected; verify credentials or complete the login challenge"
    );
    account(session)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Institution {
    pub id: u64,
    pub name: String,
}

pub fn institutions(session: &Session, query: &str) -> Result<Vec<Institution>> {
    ensure!(
        !query.trim().is_empty(),
        "institution search text is required"
    );
    let value = session
        .get(
            "https://passport2.chaoxing.com/org/searchUnis",
            &[
                ("filter".into(), query.trim().into()),
                ("product".into(), "44".into()),
                ("type".into(), "".into()),
            ],
        )?
        .json()?;
    ensure!(
        value["result"].as_bool() == Some(true),
        "institution search rejected"
    );
    value["froms"]
        .as_array()
        .context("missing institution results")?
        .iter()
        .map(|item| {
            Ok(Institution {
                id: number(&item["schoolid"])
                    .filter(|id| *id > 0)
                    .context("invalid institution ID")?,
                name: item["name"]
                    .as_str()
                    .filter(|name| !name.trim().is_empty())
                    .context("missing institution name")?
                    .into(),
            })
        })
        .collect()
}

fn login_context(session: &Session, kind: &str, fid: &str) -> Result<BTreeMap<String, String>> {
    let response = session.get_with_headers(
        "https://passport2.chaoxing.com/login",
        &[
            ("loginType".into(), kind.into()),
            ("newversion".into(), "true".into()),
            ("fid".into(), fid.into()),
        ],
        &form(&[("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/107.0.0.0 Safari/537.36")]),
    )?;
    let html = Html::parse_document(&response.text()?);
    let selector = Selector::parse("input[type=hidden][id]").unwrap();
    let fields: BTreeMap<String, String> = html
        .select(&selector)
        .filter_map(|element| {
            Some((
                element.value().attr("id")?.into(),
                element.value().attr("value").unwrap_or("").into(),
            ))
        })
        .collect();
    ensure!(
        fields.get("fid").is_some_and(|value| value == fid),
        "login page institution mismatch"
    );
    Ok(fields)
}

fn phone_input(phone: &str, country_code: &str) -> Result<()> {
    ensure!(
        (5..=11).contains(&phone.len()) && phone.bytes().all(|b| b.is_ascii_digit()),
        "invalid phone number"
    );
    ensure!(
        (1..=4).contains(&country_code.len()) && country_code.bytes().all(|b| b.is_ascii_digit()),
        "invalid country code"
    );
    ensure!(
        country_code != "86" || phone.len() == 11,
        "invalid Chinese phone number"
    );
    Ok(())
}

pub fn request_sms(
    session: &Session,
    phone: &str,
    country_code: &str,
    validate: Option<&str>,
) -> Result<()> {
    phone_input(phone, country_code)?;
    let context = login_context(session, "2", "-1")?;
    let validate = validate.unwrap_or("");
    ensure!(
        !validate.trim().is_empty() || context.get("captchaFlag").is_some_and(|v| v == "true"),
        "action-required: complete the official login captcha and supply its validate token"
    );
    let limit = session
        .post_form(
            "https://passport2.chaoxing.com/num/booleanCode",
            &[("key".into(), phone.into()), ("type".into(), "1".into())],
            &BTreeMap::new(),
        )?
        .text()?;
    ensure!(limit.trim() != "alert", "SMS daily limit reached");
    // This GET sends an SMS; never retry or follow redirects automatically.
    let response = session.get_no_redirect(
        "https://passport2.chaoxing.com/num/phonecode",
        &[
            ("phone".into(), phone.into()),
            ("code".into(), "".into()),
            ("type".into(), "1".into()),
            ("needcode".into(), "false".into()),
            ("countrycode".into(), country_code.into()),
            ("validate".into(), validate.into()),
            ("fid".into(), "-1".into()),
        ],
    )?;
    ensure!(
        response.status == 200 && response.json()?["result"].as_bool() == Some(true),
        "SMS request rejected; no message delivery confirmed"
    );
    Ok(())
}

fn login_fields(context: &BTreeMap<String, String>, keys: &[&str]) -> BTreeMap<String, String> {
    keys.iter()
        .filter_map(|key| {
            context
                .get(*key)
                .map(|value| ((*key).into(), value.clone()))
        })
        .collect()
}

fn successful_login(session: &Session, value: &Value, institution: bool) -> Result<Account> {
    ensure!(
        value["status"].as_bool() == Some(true),
        "login rejected; verify credentials or complete the login challenge"
    );
    ensure!(
        value["containTwoFactorLogin"].as_bool() != Some(true),
        "action-required: complete official two-factor login"
    );
    ensure!(
        !institution || number(&value["type"]) == Some(0),
        "action-required: institution login requires additional verification or profile completion"
    );
    account(session)
}

pub fn login_sms(session: &Session, phone: &str, code: &str) -> Result<Account> {
    phone_input(phone, "1")?;
    ensure!(
        code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit()),
        "SMS code must contain six digits"
    );
    let context = login_context(session, "2", "-1")?;
    let mut fields = login_fields(
        &context,
        &["fid", "refer", "doubleFactorLogin", "independentNameId"],
    );
    fields.insert("uname".into(), phone.into());
    // The official login script URI-encodes AES ciphertext before form serialization.
    let encoded = encrypt_login(code)?
        .replace('+', "%2B")
        .replace('/', "%2F")
        .replace('=', "%3D");
    fields.insert("verCode".into(), encoded);
    let value = session
        .post_form(
            "https://passport2.chaoxing.com/fanyaloginbycode",
            &[],
            &fields,
        )?
        .json()?;
    successful_login(session, &value, false)
}

pub fn login_student(
    session: &Session,
    fid: u64,
    student_id: &str,
    password: &str,
    validate: Option<&str>,
) -> Result<Account> {
    ensure!(
        fid > 0 && !student_id.trim().is_empty() && !password.is_empty(),
        "institution, student ID and password are required"
    );
    let context = login_context(session, "3", &fid.to_string())?;
    ensure!(
        context.get("t").is_some_and(|v| v == "true"),
        "institution login does not advertise supported credential encryption"
    );
    let validate = validate.unwrap_or("");
    ensure!(
        context.get("needVcode").is_none_or(|v| v != "1") || !validate.trim().is_empty(),
        "action-required: complete the official institution login captcha"
    );
    let mut fields = login_fields(
        &context,
        &[
            "pid",
            "fid",
            "refer",
            "t",
            "hidecompletephone",
            "doubleFactorLogin",
            "forbidotherlogin",
            "independentId",
            "independentNameId",
        ],
    );
    fields.insert("uname".into(), encrypt_login(student_id.trim())?);
    fields.insert("password".into(), encrypt_login(password)?);
    fields.insert("validate".into(), validate.into());
    let value = session
        .post_form("https://passport2.chaoxing.com/unitlogin", &[], &fields)?
        .json()?;
    successful_login(session, &value, true)
}

pub fn qr_begin(session: &Session) -> Result<QrLogin> {
    session.clear_cookies()?;
    let headers = form(&[(
        "User-Agent",
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/107.0.0.0 Safari/537.36",
    )]);
    let response =
        session.get_with_headers("https://passport2.chaoxing.com/login", &[], &headers)?;
    let html = Html::parse_document(&response.text()?);
    let read = |id: &str| -> Result<String> {
        let selector = Selector::parse(&format!("input#{id}"))
            .map_err(|_| anyhow::anyhow!("invalid login selector"))?;
        html.select(&selector)
            .next()
            .and_then(|element| element.value().attr("value"))
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .context("login page is missing QR parameters")
    };
    let uuid = read("uuid")?;
    let enc = read("enc")?;
    session.get(
        "https://passport2.chaoxing.com/createqr",
        &[("uuid".into(), uuid.clone()), ("fid".into(), "-1".into())],
    )?;
    let url = Url::parse_with_params(
        "https://passport2.chaoxing.com/toauthlogin",
        &[
            ("uuid", uuid.as_str()),
            ("enc", enc.as_str()),
            ("xxtrefer", ""),
            ("clientid", ""),
            ("mobiletip", ""),
        ],
    )?
    .to_string();
    Ok(QrLogin { uuid, enc, url })
}

pub fn qr_poll(session: &Session, qr: &QrLogin) -> Result<Option<Account>> {
    let value = session
        .post_form(
            "https://passport2.chaoxing.com/getauthstatus",
            &[],
            &form(&[("enc", &qr.enc), ("uuid", &qr.uuid)]),
        )?
        .json()?;
    if value["status"].as_bool() == Some(true) {
        return Ok(Some(account(session)?));
    }
    match value["type"].as_str() {
        Some("1") => bail!("QR login verification rejected"),
        Some("2") => bail!("QR login expired; generate another code"),
        _ => {}
    }
    if value["status"].as_bool() == Some(false) {
        return Ok(None);
    }
    bail!("unrecognized QR login response")
}

pub fn solve_captcha(session: &Session, code: &str) -> Result<()> {
    ensure!(
        !code.trim().is_empty() && !code.contains(['\r', '\n']),
        "verification code is required"
    );
    let response = session.post_form_no_redirect(
        "https://mooc1-api.chaoxing.com/html/processVerify.ac",
        &[],
        &form(&[("app", "0"), ("ucode", code)]),
    )?;
    ensure!(response.status == 302, "verification code rejected");
    Ok(())
}

pub fn fetch_captcha(session: &Session) -> Result<Vec<u8>> {
    let response = session.get(
        "https://mooc1-api.chaoxing.com/processVerifyPng.ac",
        &[("t".into(), timestamp().to_string())],
    )?;
    ensure!(
        response
            .headers
            .get("content-type")
            .is_some_and(|value| value.starts_with("image/png")),
        "invalid captcha image response"
    );
    Ok(response.body)
}

pub fn fetch_face(session: &Session, uid: u64) -> Result<Option<String>> {
    let params = inf_enc_sign(&[
        (
            "enc".into(),
            format!("{:x}", md5::compute(format!("{uid}uWwjeEKsri"))),
        ),
        ("token".into(), "4faa8662c59590c6f43ae9fe5b002b42".into()),
        ("_time".into(), timestamp().to_string()),
    ]);
    let value = session
        .get(
            "https://passport2-api.chaoxing.com/api/getUserFaceid",
            &params,
        )?
        .json()?;
    ensure!(
        number(&value["result"]) == Some(1),
        "could not retrieve registered face image"
    );
    let url = value["data"]["http"].as_str().filter(|url| !url.is_empty());
    if let Some(url) = url {
        ensure!(
            Url::parse(url)?.scheme() == "https",
            "registered image requires HTTPS"
        );
    }
    Ok(url.map(str::to_owned))
}

pub fn upload_face(session: &Session, uid: u64, image: &Path) -> Result<String> {
    let token = session
        .get("https://pan-yz.chaoxing.com/api/token/uservalid", &[])?
        .json()?;
    ensure!(
        token["result"].as_bool() == Some(true),
        "face upload token rejected"
    );
    let token = token["_token"]
        .as_str()
        .filter(|v| !v.is_empty())
        .context("missing face upload token")?;
    let value = session
        .post_multipart(
            "https://pan-yz.chaoxing.com/upload",
            &[
                ("uploadtype".into(), "face".into()),
                ("_token".into(), token.into()),
                ("puid".into(), uid.to_string()),
            ],
            &BTreeMap::new(),
            "file",
            image,
        )?
        .json()?;
    ensure!(
        value["result"].as_bool() == Some(true),
        "face upload rejected"
    );
    value["objectId"]
        .as_str()
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .context("missing face upload object ID")
}

pub fn verify_course_face(
    session: &Session,
    course_id: u64,
    class_id: u64,
    chapter_id: u64,
    cpi: u64,
    object_id: &str,
) -> Result<Value> {
    ensure!(!object_id.is_empty(), "face object ID is required");
    let value = session
        .get(
            "https://mooc1-api.chaoxing.com/mooc-ans/facephoto/clientfacecheckstatus",
            &[
                ("courseId".into(), course_id.to_string()),
                ("clazzId".into(), class_id.to_string()),
                ("cpi".into(), cpi.to_string()),
                ("chapterId".into(), chapter_id.to_string()),
                ("objectId".into(), object_id.into()),
                ("type".into(), "1".into()),
            ],
        )?
        .json()?;
    ensure!(
        value["status"].as_bool() == Some(true),
        "course face verification rejected"
    );
    Ok(value)
}

pub fn compare_exam_face(
    session: &Session,
    exam_id: u64,
    course_id: u64,
    class_id: u64,
    cpi: u64,
    object_id: &str,
    live_status: Option<u8>,
) -> Result<Value> {
    ensure!(!object_id.is_empty(), "face object ID is required");
    let live_status = live_status
        .context("action-required: provide the live detection result from the official client")?;
    let value = session
        .get(
            "https://mooc1-api.chaoxing.com/exam-ans/exam/phone/face-compare",
            &[
                ("relationid".into(), exam_id.to_string()),
                ("courseId".into(), course_id.to_string()),
                ("classId".into(), class_id.to_string()),
                ("currentFaceId".into(), object_id.into()),
                ("liveDetectionStatus".into(), live_status.to_string()),
                ("cpi".into(), cpi.to_string()),
            ],
        )?
        .json()?;
    ensure!(
        value["status"].as_bool() == Some(true),
        "exam face comparison rejected"
    );
    let data = value
        .get("data")
        .filter(|v| v.is_object())
        .context("missing exam face result")?;
    ensure!(
        data["facekey"].as_str().is_some_and(|v| !v.is_empty()),
        "exam face comparison did not issue a verification key"
    );
    Ok(data.clone())
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ImageCaptcha {
    pub captcha_id: String,
    pub kind: String,
    pub referer: String,
    pub token: String,
    pub iv: String,
    pub shade_image: String,
    pub cutout_image: String,
}

fn jsonp(response: &str) -> Result<Value> {
    let value = response
        .trim()
        .trim_end_matches(';')
        .strip_prefix("cx_captcha_function(")
        .and_then(|v| v.strip_suffix(')'))
        .context("invalid graphical captcha response")?;
    serde_json::from_str(value).map_err(|_| anyhow::anyhow!("invalid graphical captcha JSON"))
}

pub fn image_captcha_begin(
    session: &Session,
    captcha_id: &str,
    kind: &str,
    referer: &str,
) -> Result<ImageCaptcha> {
    ensure!(
        !captcha_id.is_empty()
            && matches!(
                kind,
                "slide" | "textclick" | "rotate" | "iconclick" | "obstacle"
            ),
        "invalid graphical captcha type"
    );
    ensure!(
        Url::parse(referer)?.scheme() == "https",
        "captcha referer requires HTTPS"
    );
    let headers = form(&[("Referer", referer)]);
    let conf = session.get_with_headers(
        "https://captcha.chaoxing.com/captcha/get/conf",
        &[
            ("callback".into(), "cx_captcha_function".into()),
            ("captchaId".into(), captcha_id.into()),
            ("_".into(), timestamp().to_string()),
        ],
        &headers,
    )?;
    let time = number(&jsonp(&conf.text()?)?["t"]).context("missing captcha server time")?;
    let key = format!("{:x}", md5::compute(format!("{time}{}", uuid_v4())));
    let iv = format!(
        "{:x}",
        md5::compute(format!("{captcha_id}{kind}{}{}", timestamp(), uuid_v4()))
    );
    let expires = time
        .checked_add(300000)
        .context("invalid captcha server time")?;
    let token = format!(
        "{:x}:{}",
        md5::compute(format!("{time}{captcha_id}{kind}{key}")),
        expires
    );
    let response = session.get_with_headers(
        "https://captcha.chaoxing.com/captcha/get/verification/image",
        &[
            ("callback".into(), "cx_captcha_function".into()),
            ("captchaId".into(), captcha_id.into()),
            ("type".into(), kind.into()),
            ("version".into(), "1.1.20".into()),
            ("captchaKey".into(), key),
            ("token".into(), token),
            ("referer".into(), referer.into()),
            ("iv".into(), iv.clone()),
            ("_".into(), timestamp().to_string()),
        ],
        &headers,
    )?;
    let value = jsonp(&response.text()?)?;
    let required = |field: &Value| {
        field
            .as_str()
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .context("missing graphical captcha fields")
    };
    Ok(ImageCaptcha {
        captcha_id: captcha_id.into(),
        kind: kind.into(),
        referer: referer.into(),
        token: required(&value["token"])?,
        iv,
        shade_image: required(&value["imageVerificationVo"]["shadeImage"])?,
        cutout_image: required(&value["imageVerificationVo"]["cutoutImage"])?,
    })
}

pub fn image_captcha_submit(
    session: &Session,
    challenge: &ImageCaptcha,
    coordinates: &Value,
) -> Result<String> {
    ensure!(
        coordinates.as_array().is_some_and(|v| !v.is_empty()),
        "manual captcha coordinates are required"
    );
    let response = session.get_with_headers(
        "https://captcha.chaoxing.com/captcha/check/verification/result",
        &[
            ("callback".into(), "cx_captcha_function".into()),
            ("captchaId".into(), challenge.captcha_id.clone()),
            ("type".into(), challenge.kind.clone()),
            ("token".into(), challenge.token.clone()),
            ("textClickArr".into(), serde_json::to_string(coordinates)?),
            ("coordinate".into(), "[]".into()),
            ("runEnv".into(), "10".into()),
            ("version".into(), "1.1.20".into()),
            ("t".into(), "a".into()),
            ("iv".into(), challenge.iv.clone()),
            ("_".into(), timestamp().to_string()),
        ],
        &form(&[("Referer", &challenge.referer)]),
    )?;
    let value = jsonp(&response.text()?)?;
    ensure!(
        value["result"].as_bool() == Some(true),
        "manual graphical captcha verification rejected"
    );
    let extra: Value = serde_json::from_str(
        value["extraData"]
            .as_str()
            .context("missing captcha validation result")?,
    )
    .map_err(|_| anyhow::anyhow!("invalid captcha validation result"))?;
    extra["validate"]
        .as_str()
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .context("missing captcha validation token")
}

fn uuid_v4() -> String {
    let mut bytes: [u8; 16] = rand::random();
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::test_server;

    fn login_page(fid: &str, captcha: bool) -> Vec<u8> {
        format!(r#"<input type="hidden" id="fid" value="{fid}"><input type="hidden" id="pid" value="-1"><input type="hidden" id="t" value="true"><input type="hidden" id="refer" value="fixture"><input type="hidden" id="doubleFactorLogin" value="0"><input type="hidden" id="captchaFlag" value="{}"><input type="hidden" id="needVcode" value="{}">"#, !captcha, if captcha { "1" } else { "" }).into_bytes()
    }

    fn request_fields(request: &[u8]) -> BTreeMap<String, String> {
        let request = String::from_utf8_lossy(request);
        let body = request.split_once("\r\n\r\n").unwrap().1;
        Url::parse(&format!("https://fixture.invalid/?{body}"))
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect()
    }

    #[test]
    fn institution_lookup_keeps_duplicate_names_with_distinct_ids() {
        let (url, server) = test_server(vec![("Content-Type: application/json\r\n", br#"{"result":true,"froms":[{"schoolid":780,"name":"School"},{"schoolid":"39459","name":"School"}]}"#.to_vec())]);
        let session = Session::new(2, 0, Some(&url)).unwrap();
        let schools = institutions(&session, "School").unwrap();
        assert_eq!(
            schools.iter().map(|v| v.id).collect::<Vec<_>>(),
            vec![780, 39459]
        );
        let requests = server.join().unwrap();
        let request = String::from_utf8_lossy(&requests[0]);
        assert!(request.starts_with("GET /org/searchUnis?filter=School&product=44&type= "));
    }

    #[test]
    fn sms_request_uses_manual_challenge_and_single_delivery_request() {
        let (url, server) = test_server(vec![
            ("Content-Type: text/html\r\n", login_page("-1", true)),
            ("Content-Type: text/plain\r\n", b"ok".to_vec()),
            (
                "Content-Type: application/json\r\n",
                br#"{"result":true}"#.to_vec(),
            ),
        ]);
        let session = Session::new(2, 3, Some(&url)).unwrap();
        request_sms(&session, "13800000000", "86", Some("manual-token")).unwrap();
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(
            String::from_utf8_lossy(&requests[1])
                .starts_with("POST /num/booleanCode?key=13800000000&type=1 ")
        );
        let send = String::from_utf8_lossy(&requests[2]);
        assert!(send.starts_with("GET /num/phonecode?"));
        assert!(send.contains("needcode=false&countrycode=86&validate=manual-token&fid=-1"));
    }

    #[test]
    fn sms_challenge_and_quota_fail_before_delivery() {
        let (url, server) = test_server(vec![(
            "Content-Type: text/html\r\n",
            login_page("-1", true),
        )]);
        let session = Session::new(2, 0, Some(&url)).unwrap();
        assert!(
            request_sms(&session, "13800000000", "86", None)
                .unwrap_err()
                .to_string()
                .contains("action-required")
        );
        assert_eq!(server.join().unwrap().len(), 1);
        let (url, server) = test_server(vec![
            ("Content-Type: text/html\r\n", login_page("-1", false)),
            ("Content-Type: text/plain\r\n", b"alert".to_vec()),
        ]);
        let session = Session::new(2, 0, Some(&url)).unwrap();
        assert!(request_sms(&session, "13800000000", "86", None).is_err());
        assert_eq!(server.join().unwrap().len(), 2);
    }

    #[test]
    fn sms_login_encrypts_code_with_the_official_double_encoding() {
        let (url, server) = test_server(vec![
            ("Content-Type: text/html\r\n", login_page("-1", true)),
            (
                "Content-Type: application/json\r\n",
                br#"{"status":true}"#.to_vec(),
            ),
            ("Content-Type: application/json\r\n", account_body()),
        ]);
        let session = Session::new(2, 0, Some(&url)).unwrap();
        assert_eq!(
            login_sms(&session, "13800000000", "123456").unwrap().puid,
            123
        );
        let requests = server.join().unwrap();
        assert!(String::from_utf8_lossy(&requests[1]).starts_with("POST /fanyaloginbycode "));
        let fields = request_fields(&requests[1]);
        assert_eq!(fields["uname"], "13800000000");
        assert_eq!(
            fields["verCode"],
            encrypt_login("123456")
                .unwrap()
                .replace('+', "%2B")
                .replace('/', "%2F")
                .replace('=', "%3D")
        );
        assert!(!fields["verCode"].contains("123456"));
    }

    #[test]
    fn student_login_encrypts_credentials_and_rejects_incomplete_receipts() {
        let (url, server) = test_server(vec![
            ("Content-Type: text/html\r\n", login_page("780", true)),
            (
                "Content-Type: application/json\r\n",
                br#"{"status":true,"type":0}"#.to_vec(),
            ),
            ("Content-Type: application/json\r\n", account_body()),
        ]);
        let session = Session::new(2, 0, Some(&url)).unwrap();
        assert_eq!(
            login_student(&session, 780, "student-1", "secret", Some("manual-token"))
                .unwrap()
                .puid,
            123
        );
        let requests = server.join().unwrap();
        let fields = request_fields(&requests[1]);
        assert_eq!(fields["fid"], "780");
        assert_eq!(fields["uname"], encrypt_login("student-1").unwrap());
        assert_eq!(fields["password"], encrypt_login("secret").unwrap());
        assert_eq!(fields["validate"], "manual-token");
        for receipt in [
            r#"{"status":true,"type":2}"#,
            r#"{"status":true,"type":0,"containTwoFactorLogin":true}"#,
            r#"{"status":false,"type":0}"#,
        ] {
            let (url, server) = test_server(vec![
                ("Content-Type: text/html\r\n", login_page("780", false)),
                (
                    "Content-Type: application/json\r\n",
                    receipt.as_bytes().to_vec(),
                ),
            ]);
            let session = Session::new(2, 0, Some(&url)).unwrap();
            assert!(login_student(&session, 780, "student-1", "secret", None).is_err());
            assert_eq!(server.join().unwrap().len(), 2);
        }
    }

    #[test]
    fn manual_captcha_rejects_invalid_jsonp_and_empty_inputs() {
        assert_eq!(
            jsonp("cx_captcha_function({\"t\":123});").unwrap(),
            json!({"t": 123})
        );
        assert!(jsonp("other({\"secret\":\"do not echo\"})").is_err());
        let session = Session::new(2, 0, Some("http://127.0.0.1:9999")).unwrap();
        assert!(solve_captcha(&session, "").is_err());
        assert!(login_password(&session, "", "").is_err());
    }

    fn account_body() -> Vec<u8> {
        serde_json::to_vec(&json!({"result":1,"msg":{"puid":"123","name":"Fixture User","phone":"13800000000","schoolname":"Fixture School","sex":"1","uname":"student-1"}})).unwrap()
    }

    #[test]
    fn password_login_encrypts_credentials_and_fetches_account_with_the_cookie() {
        let (url, server) = test_server(vec![
            (
                "Content-Type: application/json\r\nSet-Cookie: authenticated=yes; Path=/\r\n",
                br#"{"status":true}"#.to_vec(),
            ),
            ("Content-Type: application/json\r\n", account_body()),
        ]);
        let session = Session::new(2, 0, Some(&url)).unwrap();
        let result = login_password(&session, "13800000000", "fixture-password").unwrap();
        assert_eq!(result.puid, 123);
        assert_eq!(result.stu_id.as_deref(), Some("student-1"));
        let requests = server.join().unwrap();
        let login = String::from_utf8(requests[0].clone()).unwrap();
        assert!(login.starts_with("POST /fanyalogin "));
        assert!(!login.contains("13800000000") && !login.contains("fixture-password"));
        let body = login.split_once("\r\n\r\n").unwrap().1;
        let parsed = Url::parse(&format!("https://fixture.invalid/?{body}"))
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect::<BTreeMap<_, _>>();
        assert_eq!(parsed["uname"], "ZBjZ8C7FsVyCJ12TKWWRSQ==");
        assert_eq!(
            parsed["password"],
            encrypt_login("fixture-password").unwrap()
        );
        assert_eq!(parsed["fid"], "-1");
        assert_eq!(parsed["t"], "true");
        assert_eq!(parsed["forbidotherlogin"], "0");
        assert_eq!(parsed["validate"], "");
        let account_request = String::from_utf8(requests[1].clone()).unwrap();
        assert!(account_request.starts_with("GET /apis/login/userLogin4Uname.do "));
        assert!(
            account_request
                .to_ascii_lowercase()
                .contains("cookie: authenticated=yes")
        );
    }

    #[test]
    fn qr_begin_polls_until_account_and_rejects_expired_codes() {
        let (url, server) = test_server(vec![
            (
                "Content-Type: text/html\r\n",
                br#"<input id="uuid" value="fixture-uuid"><input id="enc" value="fixture-enc">"#
                    .to_vec(),
            ),
            ("Content-Type: image/png\r\n", b"fixture-image".to_vec()),
            (
                "Content-Type: application/json\r\n",
                br#"{"status":false,"type":"4"}"#.to_vec(),
            ),
            (
                "Content-Type: application/json\r\nSet-Cookie: authenticated=yes; Path=/\r\n",
                br#"{"status":true}"#.to_vec(),
            ),
            ("Content-Type: application/json\r\n", account_body()),
            (
                "Content-Type: application/json\r\n",
                br#"{"status":false,"type":"2"}"#.to_vec(),
            ),
        ]);
        let session = Session::new(2, 0, Some(&url)).unwrap();
        session.import_cookie("UID=prior-session").unwrap();
        let qr = qr_begin(&session).unwrap();
        assert!(session.cookie_value("UID").is_none());
        assert_eq!(qr.uuid, "fixture-uuid");
        assert_eq!(qr.enc, "fixture-enc");
        let query = Url::parse(&qr.url)
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect::<BTreeMap<_, _>>();
        assert_eq!(query["uuid"], "fixture-uuid");
        assert_eq!(query["enc"], "fixture-enc");
        assert!(qr_poll(&session, &qr).unwrap().is_none());
        assert_eq!(qr_poll(&session, &qr).unwrap().unwrap().puid, 123);
        assert!(
            qr_poll(&session, &qr)
                .unwrap_err()
                .to_string()
                .contains("expired")
        );
        let requests = server.join().unwrap();
        let login = String::from_utf8_lossy(&requests[0]);
        assert!(login.starts_with("GET /login "));
        assert!(
            login
                .to_ascii_lowercase()
                .contains("user-agent: mozilla/5.0")
        );
        assert!(
            String::from_utf8_lossy(&requests[1])
                .starts_with("GET /createqr?uuid=fixture-uuid&fid=-1 ")
        );
        for index in [2, 3, 5] {
            let request = String::from_utf8_lossy(&requests[index]);
            assert!(request.starts_with("POST /getauthstatus "));
            assert!(request.ends_with("enc=fixture-enc&uuid=fixture-uuid"));
        }
        assert!(
            String::from_utf8_lossy(&requests[4])
                .to_ascii_lowercase()
                .contains("cookie: authenticated=yes")
        );
    }
}
