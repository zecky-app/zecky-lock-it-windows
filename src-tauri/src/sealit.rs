//! Seal it on Windows: pairing a phone and sending with its signature, by
//! QR codes both ways (the webcam reads the phone's codes).

use std::sync::Mutex;

use lockit_wallet::sealit::{ScanProgress, UrScan, UR_ACCOUNTS, UR_PCZT};
use lockit_wallet::Wallet;
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::{
    problem,
    wallet::{self as w, MadeIn, Sent, WalletSlot},
    CommandResult,
};

/// Seal it's moving parts while a pairing or payment is under way.
#[derive(Default)]
pub struct SealSlot {
    scan: Mutex<Option<UrScan>>,
    /// The full PCZT and, once made, its proofs (made while the phone signs).
    proving: Mutex<Option<tauri::async_runtime::JoinHandle<CommandResult<Vec<u8>>>>>,
}

/// Starts reading the phone's codes: "accounts" (pairing) or "pczt" (the
/// signed payment).
#[tauri::command]
pub fn sealit_scan_start(seal: State<'_, SealSlot>, kind: String) -> CommandResult<()> {
    let expected = match kind.as_str() {
        "accounts" => UR_ACCOUNTS,
        "pczt" => UR_PCZT,
        _ => return Err(problem("unknown code")),
    };
    *seal.scan.lock().map_err(|_| problem("the app is in an unknown state"))? = Some(UrScan::new(expected));
    Ok(())
}

/// One code read by the webcam (or pasted).
#[tauri::command]
pub fn sealit_scan_push(seal: State<'_, SealSlot>, text: String) -> CommandResult<ScanProgress> {
    let mut scan = seal.scan.lock().map_err(|_| problem("the app is in an unknown state"))?;
    let scan = scan.as_mut().ok_or_else(|| problem("start the scan again"))?;
    Ok(scan.receive(&text)?)
}

fn scanned(seal: &SealSlot) -> CommandResult<Vec<u8>> {
    seal.scan
        .lock()
        .map_err(|_| problem("the app is in an unknown state"))?
        .take()
        .and_then(|scan| scan.message().map(<[u8]>::to_vec))
        .ok_or_else(|| problem("the code was not read completely; scan it again"))
}

/// Pairs the phone whose account code was scanned. `made_in`: when its
/// words were made; none for a brand-new phone wallet (starts near now).
#[tauri::command]
pub async fn sealit_pair(app: AppHandle, slot: State<'_, WalletSlot>, seal: State<'_, SealSlot>, made_in: Option<MadeIn>) -> CommandResult<String> {
    let message = scanned(&seal)?;
    let export = lockit_wallet::sealit::parse_accounts(&message)?;
    let name = export.accounts[0].name.clone().unwrap_or_else(|| "Seal it".into());
    let main = w::main_wallet(&app)?;
    let server = w::chosen_server(&app);
    let id = tauri::async_runtime::spawn_blocking(move || -> CommandResult<String> {
        let tip = w::runtime().block_on(Wallet::chain_tip(&server))?;
        let birthday = match made_in {
            Some(made) => lockit_wallet::estimate_birthday(made.year, made.month, tip, w::now()),
            None => tip.saturating_sub(1_000),
        };
        Ok(w::runtime().block_on(main.add_sealit(&name, &export, Some(birthday), &server))?)
    })
    .await
    .map_err(|e| problem(format!("pairing stopped unexpectedly: {e}")))??;
    w::select(&app, Some(id.clone()))?;
    w::nudge_slot(&slot);
    Ok(id)
}

/// The payment, as QR codes for the phone.
#[derive(Serialize)]
pub struct Unsigned {
    /// One SVG per frame; more than one is an animated code.
    frames: Vec<String>,
}

/// Seal it, step 1: turns the worked-out payment into codes for the phone,
/// and starts the proofs here while the phone signs.
#[tauri::command]
pub async fn sealit_start(app: AppHandle, slot: State<'_, WalletSlot>, seal: State<'_, SealSlot>) -> CommandResult<Unsigned> {
    let payment = w::take_prepared(&slot)?;
    let wallet = w::wallet(&app)?;
    let (full, frames) = tauri::async_runtime::spawn_blocking(move || -> CommandResult<(Vec<u8>, Vec<String>)> {
        let (full, parts) = wallet.seal_start(&payment)?;
        let frames = parts.iter().map(|p| lockit_wallet::qr_svg(p)).collect::<Result<Vec<_>, _>>()?;
        Ok((full, frames))
    })
    .await
    .map_err(|e| problem(format!("building the payment stopped unexpectedly: {e}")))??;
    let proving = tauri::async_runtime::spawn_blocking(move || Ok(Wallet::seal_prove(&full)?));
    if let Some(old) = seal.proving.lock().map_err(|_| problem("the app is in an unknown state"))?.replace(proving) {
        old.abort();
    }
    Ok(Unsigned { frames })
}

/// Seal it, step 3: the scanned signature and this PC's proofs together,
/// handed to the network.
#[tauri::command]
pub async fn sealit_finish(app: AppHandle, slot: State<'_, WalletSlot>, seal: State<'_, SealSlot>, over_tor: bool) -> CommandResult<Sent> {
    let message = scanned(&seal)?;
    let proving = seal
        .proving
        .lock()
        .map_err(|_| problem("the app is in an unknown state"))?
        .take()
        .ok_or_else(|| problem("no payment is waiting for this signature; start again"))?;
    w::step(&app, "Finishing the proofs");
    let proven = proving.await.map_err(|e| problem(format!("the proofs stopped unexpectedly: {e}")))??;
    let wallet = w::wallet(&app)?;
    let signed = tauri::async_runtime::spawn_blocking(move || wallet.seal_finish(&proven, &message))
        .await
        .map_err(|e| problem(e.to_string()))??;
    w::step(&app, if over_tor { "Connecting over Tor" } else { "Handing it to the network" });
    w::broadcast(&app, &slot, signed, over_tor).await
}

/// Stops a Seal it payment that was not finished.
#[tauri::command]
pub fn sealit_cancel(seal: State<'_, SealSlot>) {
    if let Ok(mut proving) = seal.proving.lock() {
        if let Some(job) = proving.take() {
            job.abort();
        }
    }
    if let Ok(mut scan) = seal.scan.lock() {
        *scan = None;
    }
}
