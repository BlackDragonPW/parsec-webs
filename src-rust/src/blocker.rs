// src-rust/src/blocker.rs — Parsec Android ad/tracker blocker
//
// Ported from desktop blocker.rs — same block-lists, same logic.
// On Android, shouldBlockResource() is called from WebViewClient.shouldInterceptRequest()
// which fires for every subresource (JS, CSS, images, XHR, fetch).
// This gives engine-level blocking equivalent to WKContentRuleList on desktop.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock};
use serde::{Deserialize, Serialize};
use crate::BrowserPrefs;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct BlockStats {
    pub ads_blocked:      u64,
    pub trackers_blocked: u64,
    pub popups_blocked:   u64,
    pub nsfw_blocked:     u64,
    pub miners_blocked:   u64,
    pub bytes_saved:      u64,
    pub requests_total:   u64,
}

static ADS_BLOCKED:      AtomicU64 = AtomicU64::new(0);
static TRK_BLOCKED:      AtomicU64 = AtomicU64::new(0);
static POPUPS_BLOCKED:   AtomicU64 = AtomicU64::new(0);
static NSFW_BLOCKED:     AtomicU64 = AtomicU64::new(0);
static MINERS_BLOCKED:   AtomicU64 = AtomicU64::new(0);
static BYTES_SAVED:      AtomicU64 = AtomicU64::new(0);
static REQUESTS_TOTAL:   AtomicU64 = AtomicU64::new(0);

static AD_HOSTS: OnceLock<HashSet<String>>  = OnceLock::new();
static TR_HOSTS: OnceLock<HashSet<String>>  = OnceLock::new();

pub fn init() {
    AD_HOSTS.get_or_init(|| build_ad_hosts());
    TR_HOSTS.get_or_init(|| build_tracker_hosts());
    tracing::info!("Blocker: {} ad hosts, {} tracker hosts",
        AD_HOSTS.get().map(|h| h.len()).unwrap_or(0),
        TR_HOSTS.get().map(|h| h.len()).unwrap_or(0));
}

/// JSON block-lists for Kotlin-side hot-path blocking (loaded once at startup).
pub fn block_lists_json() -> String {
    init();
    let ads: Vec<&String> = AD_HOSTS.get().map(|h| h.iter().collect()).unwrap_or_default();
    let trk: Vec<&String> = TR_HOSTS.get().map(|h| h.iter().collect()).unwrap_or_default();
    serde_json::json!({ "ads": ads, "trackers": trk }).to_string()
}

/// Record a block event from Kotlin (stats only — no URL parsing).
pub fn record_block(reason: &str) {
    REQUESTS_TOTAL.fetch_add(1, Ordering::Relaxed);
    match reason {
        "ads"      => { ADS_BLOCKED.fetch_add(1, Ordering::Relaxed); BYTES_SAVED.fetch_add(25_000, Ordering::Relaxed); }
        "trackers" => { TRK_BLOCKED.fetch_add(1, Ordering::Relaxed); BYTES_SAVED.fetch_add(8_000, Ordering::Relaxed); }
        "nsfw"     => { NSFW_BLOCKED.fetch_add(1, Ordering::Relaxed); }
        "miners"   => { MINERS_BLOCKED.fetch_add(1, Ordering::Relaxed); }
        "popups"   => { POPUPS_BLOCKED.fetch_add(1, Ordering::Relaxed); }
        _ => {}
    }
}

pub fn get_stats() -> BlockStats {
    BlockStats {
        ads_blocked:      ADS_BLOCKED.load(Ordering::Relaxed),
        trackers_blocked: TRK_BLOCKED.load(Ordering::Relaxed),
        popups_blocked:   POPUPS_BLOCKED.load(Ordering::Relaxed),
        nsfw_blocked:     NSFW_BLOCKED.load(Ordering::Relaxed),
        miners_blocked:   MINERS_BLOCKED.load(Ordering::Relaxed),
        bytes_saved:      BYTES_SAVED.load(Ordering::Relaxed),
        requests_total:   REQUESTS_TOTAL.load(Ordering::Relaxed),
    }
}

pub fn reset_stats() {
    ADS_BLOCKED.store(0, Ordering::Relaxed);
    TRK_BLOCKED.store(0, Ordering::Relaxed);
    POPUPS_BLOCKED.store(0, Ordering::Relaxed);
    NSFW_BLOCKED.store(0, Ordering::Relaxed);
    MINERS_BLOCKED.store(0, Ordering::Relaxed);
    BYTES_SAVED.store(0, Ordering::Relaxed);
    REQUESTS_TOTAL.store(0, Ordering::Relaxed);
}

/// Returns Some("ads"|"trackers"|"nsfw"|"miners"|"popups") if the navigation URL should be blocked.
pub fn should_block(url: &str, prefs: &BrowserPrefs) -> Option<&'static str> {
    let host = extract_host(url)?;
    check_host(&host, prefs)
}

/// Returns Some(reason) if a subresource URL should be blocked.
pub fn should_block_resource(url: &str, prefs: &BrowserPrefs) -> Option<&'static str> {
    REQUESTS_TOTAL.fetch_add(1, Ordering::Relaxed);

    let host = extract_host(url)?;
    let reason = check_host(&host, prefs)?;

    match reason {
        "ads"      => { ADS_BLOCKED.fetch_add(1, Ordering::Relaxed); BYTES_SAVED.fetch_add(25_000, Ordering::Relaxed); }
        "trackers" => { TRK_BLOCKED.fetch_add(1, Ordering::Relaxed); BYTES_SAVED.fetch_add(8_000, Ordering::Relaxed); }
        "nsfw"     => { NSFW_BLOCKED.fetch_add(1, Ordering::Relaxed); }
        "miners"   => { MINERS_BLOCKED.fetch_add(1, Ordering::Relaxed); }
        "popups"   => { POPUPS_BLOCKED.fetch_add(1, Ordering::Relaxed); }
        _ => {}
    }

    Some(reason)
}

/// O(domain labels) lookup — walks host suffixes against the block set.
fn is_blocked_host(host: &str, blocked: &HashSet<String>) -> bool {
    if blocked.contains(host) {
        return true;
    }
    let mut rest = host;
    while let Some(dot) = rest.find('.') {
        rest = &rest[dot + 1..];
        if blocked.contains(rest) {
            return true;
        }
    }
    false
}

fn check_host(host: &str, prefs: &BrowserPrefs) -> Option<&'static str> {
    if prefs.block_ads {
        if AD_HOSTS.get().map(|h| is_blocked_host(host, h)).unwrap_or(false) {
            return Some("ads");
        }
    }
    if prefs.block_trackers {
        if TR_HOSTS.get().map(|h| is_blocked_host(host, h)).unwrap_or(false) {
            return Some("trackers");
        }
    }
    if prefs.block_nsfw && is_nsfw(host) {
        return Some("nsfw");
    }
    if is_miner(host) {
        return Some("miners");
    }
    None
}

fn extract_host(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    Some(u.host_str()?.to_lowercase())
}

fn is_nsfw(host: &str) -> bool {
    const NSFW: &[&str] = &["pornhub", "xvideos", "xnxx", "redtube", "youporn"];
    NSFW.iter().any(|n| host.contains(n))
}

fn is_miner(host: &str) -> bool {
    const MINERS: &[&str] = &["coinhive", "cryptoloot", "coin-hive", "minero.cc", "webmr.ru"];
    MINERS.iter().any(|m| host.contains(m))
}

// ── Block-lists ───────────────────────────────────────────────────────────────
// Expanded versions of the desktop lists. In production these would be loaded
// from bundled assets (assets/blocklists/ads.txt, trackers.txt) which are
// updated via background sync.

fn build_ad_hosts() -> HashSet<String> {
    let hosts: &[&str] = &[
        // Ad networks
        "doubleclick.net", "googlesyndication.com", "googleadservices.com",
        "ads.google.com", "pagead2.googlesyndication.com",
        "adservice.google.com", "adservice.google.co.uk", "adservice.google.de",
        "amazon-adsystem.com", "media.amazon.com", "aax-us-east.amazon.com",
        "advertising.com", "aol.com", "tacoda.net",
        "ads.yahoo.com", "ads.yimg.com",
        "adsystem.com", "adtech.com",
        "outbrain.com", "amplifyjs.net",
        "taboola.com", "cdn.taboola.com",
        "popads.net", "popcash.net",
        "propellerads.com", "prophix.com",
        "revenuehits.com", "revcontent.com",
        "exoclick.com", "trafficjunky.net",
        "trafficshop.com", "contentabc.com",
        "adnxs.com", "appnexus.com",
        "rubiconproject.com", "rubicon.com",
        "openx.net", "openx.com",
        "pubmatic.com", "media6degrees.com",
        "criteo.com", "criteo.net", "ads.criteo.com",
        "adsymptotic.com", "adipex.net",
        "adriver.ru", "admixer.net",
        "adjuggler.com", "adpinion.com",
        "admeld.com", "admob.com",
        "moatads.com", "flashtalking.com",
        "yieldmo.com", "lijit.com",
        "sovrn.com", "disqusads.com",
        "zedo.com", "undertone.com",
        "casalemedia.com", "indexww.com",
        "sharethrough.com", "tribalfusion.com",
        "brightmountainmedia.com", "adform.net",
        "smaato.net", "smartadserver.com",
        "mfadsrvr.com", "media.net",
        "33across.com", "3lift.com",
        "turn.com", "spotxchange.com",
        "spotx.tv", "yieldlab.net",
        "connatix.com", "bidswitch.net",
        "indexexchange.com", "buzzoola.com",
        "adfox.ru", "between.digital",
        "recreativ.ru", "vkontakte-ads.ru",
        "buzzoola.com", "mytarget.ru",
        "soloway.ru", "aidata.me",
        "yandex-team.ru",
        // Additional mobile ad SDKs
        "applovin.com", "mopub.com",
        "flurry.com", "unity3d.com",
        "chartboost.com", "ironsrc.com",
        "vungle.com", "inmobi.com",
        "smaato.com", "millennialmedia.com",
        "mobfox.com", "mobyaffiliates.com",
        "startapp.com", "airpush.com",
        "leadbolt.net", "taptica.com",
    ];
    hosts.iter().map(|s| s.to_string()).collect()
}

fn build_tracker_hosts() -> HashSet<String> {
    let hosts: &[&str] = &[
        // Analytics / tracking
        "google-analytics.com", "analytics.google.com",
        "ssl.google-analytics.com", "stats.g.doubleclick.net",
        "googletagmanager.com", "googletagservices.com",
        "mixpanel.com", "segment.io", "segment.com",
        "hotjar.com", "fullstory.com",
        "mouseflow.com", "crazyegg.com",
        "kissmetrics.com", "kissanalytics.com",
        "heap.io", "heapanalytics.com",
        "amplitude.com", "cdn.amplitude.com",
        "optimizely.com", "launchdarkly.com",
        "intercom.io", "intercom.com",
        "zendesk.com",
        "newrelic.com", "nr-data.net",
        "dynatrace.com", "ruxit.com",
        "pingdom.com", "speedcurve.com",
        "comscore.com", "scorecardresearch.com",
        "quantserve.com", "quantcast.com",
        "mxpnl.com",
        "branch.io", "adjust.com",
        "appsflyer.com", "kochava.com",
        "singular.net", "tenjin.io",
        "firebase.io", "app-measurement.com",
        "facebook.com/tr", "connect.facebook.net",
        "facebook.net", "fbcdn.net", "fbevents.js",
        "twitter.com/i/adsct", "static.ads-twitter.com",
        "pixel.advertising.com",
        "bat.bing.com", "clarity.ms",
        "ads.linkedin.com", "snap.licdn.com",
        "platform.linkedin.com",
        "ads-twitter.com", "analytics.twitter.com",
        "pinterest.com/ct", "ct.pinterest.com",
        "tiktok.com/i18n", "analytics.tiktok.com",
        "mc.yandex.ru", "metrika.yandex.ru",
        "counter.ok.ru", "top.mail.ru",
        "rambler.ru",
        // Fingerprinting
        "fingerprintjs.com", "fpjs.io",
        "threatmetrix.com", "iovation.com",
        "deviceatlas.com", "siftscience.com",
        // A/B testing / session replay
        "optimizely.com", "vwo.com",
        "convertexperiments.com", "logrocket.io",
        "sentry.io", "rollbar.com",
        "bugsnag.com", "raygun.io",
    ];
    hosts.iter().map(|s| s.to_string()).collect()
}
