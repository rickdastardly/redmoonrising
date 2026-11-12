use eve_marshal::Value;

use crate::services::{Call, Ctx, Outcome};
use crate::svc::chat::CHTMODE_SPEAKER;
use crate::svc::rows::{int, rowset};

const HEADER: &[&str] = &["characterID", "online"];

pub async fn online_status(call: &Call, ctx: &Ctx<'_>) -> Option<Outcome> {
    let value = match call.method.as_str() {
        "GetInitialState" => initial_state(ctx).await,
        "GetOnlineStatus" => Value::Bool(ctx.world.lock().contact_is_online(call.arg_id(0))),
        _ => return None,
    };
    Some(Outcome::Value(value))
}

async fn initial_state(ctx: &Ctx<'_>) -> Value {
    let Some(char_id) = ctx.session.char_id else { return rowset(HEADER, Vec::new()) };
    // The native LSC buddy list lives in the owner's personal channel ACL.
    let sql = r#"SELECT "accessor" FROM "channelMods" WHERE "channelID" = $1 AND "mode" >= $2"#;
    let rows = ctx.query(sql, &[&(char_id as i32), &CHTMODE_SPEAKER]).await;
    let contacts = rows.iter().filter_map(|r| int(r, 0)).filter(|&id| id > 0 && id != char_id);
    let status = ctx.world.lock().watch_contacts(ctx.client_id, contacts);
    let lines = std::iter::once((char_id, true)).chain(status)
        .map(|(id, online)| Value::List(vec![Value::Int(id), Value::Bool(online)]))
        .collect();
    rowset(HEADER, lines)
}
