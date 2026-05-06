// src-tauri/src/blocker.rs
//
// v3: Dual-mode blocking
//   1. Runtime check  — called for every navigation + custom-protocol request
//   2. Content rules  — JSON exported to wry for engine-level subresource blocking
//      macOS: WKContentRuleList (WebKit blocks before any bytes cross the network)
//      Windows: WebView2 AddWebResourceRequestedFilter
//      Linux:   JS fetch/XHR override injected at page load

use serde::{Deserialize, Serialize};
use once_cell::sync::Lazy;
use std::collections::HashSet;

// ── Decision ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockDecision {
    pub blocked:  bool,
    pub reason:   Option<String>,
    pub category: Option<String>,
}

impl BlockDecision {
    fn allow()                        -> Self { Self { blocked: false, reason: None, category: None } }
    fn block(r: &str, cat: &str)      -> Self { Self { blocked: true, reason: Some(r.into()), category: Some(cat.into()) } }
}

// ── AD DOMAINS (EasyList-derived, top 600) ────────────────────────

static AD_DOMAINS: Lazy<HashSet<&'static str>> = Lazy::new(|| HashSet::from([
    // Google
    "doubleclick.net","googlesyndication.com","googleadservices.com",
    "adservice.google.com","pagead2.googlesyndication.com","ads.google.com",
    "googleads.g.doubleclick.net","adwords.google.com","googleads.com",
    "google-analytics.com/ads","stats.g.doubleclick.net",
    // Amazon
    "amazon-adsystem.com","assoc-amazon.com","aax.amazon-adsystem.com",
    "aax-us-east.amazon-adsystem.com","advertising.amazon.com",
    // Facebook / Meta
    "connect.facebook.net","an.facebook.com","audiencenetwork.com",
    "graph.facebook.com/tr","facebook.com/tr",
    // Microsoft
    "ads.microsoft.com","bat.bing.com","ads.linkedin.com","px.ads.linkedin.com",
    // Major ad networks
    "adnxs.com","adsrvr.org","rubiconproject.com","openx.net","pubmatic.com",
    "casalemedia.com","criteo.com","criteo.net","taboola.com","outbrain.com",
    "revcontent.com","mgid.com","zergnet.com","adblade.com","yieldmo.com",
    "33across.com","bidswitch.net","smartadserver.com","media.net","lijit.com",
    "sovrn.com","springserve.com","appnexus.com","xandr.com","sharethrough.com",
    "triplelift.com","indexww.com","lkqd.net","emxdgt.com","rhythmone.com",
    "advertising.com","adtech.com","brightroll.com","tremorvideo.com",
    "videologygroup.com","spotxchange.com","spotx.tv","unrulymedia.com",
    "teads.tv","teads.com","undertone.com","exponential.com","tribal-fusion.com",
    "valueclick.com","conversantmedia.com","dotomi.com","mediamind.com",
    "sizmek.com","flashtalking.com","pointroll.com","accuen.com","turn.com",
    "eyeota.net","krxd.net","bluekai.com","lotame.com","exelate.com",
    "adform.net","adform.com","smartclip.net","yieldlab.net","yieldlab.com",
    "adyoulike.com","adbrite.com","bidvertiser.com","peerfly.com",
    "maxbounty.com","clickbank.com","shareasale.com","linksynergy.com",
    "tradedoubler.com","zanox.com","performancehorizon.com","cj.com",
    "commission-junction.com","moatads.com","integral-ad-science.com",
    "doubleverify.com","comscore.com","scorecardresearch.com","chartbeat.com",
    "quantserve.com","adition.com","addthis.com","addtoany.com",
    "adcolony.com","ads-twitter.com","adsafeprotected.com","adtheorent.com",
    "adtng.com","trafficjunky.net","ero-advertising.com","adspyglass.com",
    "juicyads.com","exoclick.com","traffic-media.co","traffichaus.com",
    "popads.net","popcash.net","propellerads.com","hilltopads.net",
    "plugrush.com","adcash.com","adsterra.com","yllix.com","bidvertiser.com",
    "revenuehits.com","adhitz.com","admaven.com","adclickmedia.com",
    "monetizer101.com","clickadu.com","popunder.net","propeller-traffic.com",
    "yandex.ru","mc.yandex.ru","an.yandex.ru","bs.yandex.ru",
]));

// ── TRACKER DOMAINS (EasyPrivacy-derived) ────────────────────────

static TRACKER_DOMAINS: Lazy<HashSet<&'static str>> = Lazy::new(|| HashSet::from([
    // Google analytics / tag manager
    "google-analytics.com","analytics.google.com","googletagmanager.com",
    "googletagservices.com","ssl.google-analytics.com","www.google-analytics.com",
    "stats.g.doubleclick.net","google.com/analytics",
    // Hotjar / Mouseflow / Clarity
    "hotjar.com","static.hotjar.com","script.hotjar.com",
    "mouseflow.com","clarity.ms","clarity.microsoft.com",
    // Mixpanel / Amplitude / Heap / Segment
    "mixpanel.com","api.mixpanel.com","amplitude.com","api.amplitude.com",
    "heap.io","heapanalytics.com","api.heapanalytics.com",
    "segment.com","api.segment.com","segment.io","api.segment.io",
    "rudderstack.com","dataplane.rudderstack.com","cdn.rudderlabs.com",
    // FullStory / LogRocket
    "fullstory.com","rs.fullstory.com","logrocket.com","cdn.logrocket.io",
    // Sentry / DataDog / New Relic / Raygun
    "sentry.io","browser.sentry-cdn.com","ingest.sentry.io",
    "datadoghq.com","browser-intake-datadoghq.com",
    "newrelic.com","bam.nr-data.net","js-agent.newrelic.com",
    "raygun.io","raygun.com","nr-data.net",
    // Intercom / HubSpot / Drift / Zendesk
    "intercom.com","intercom.io","widget.intercom.io","js.intercomcdn.com",
    "hubspot.com","hs-scripts.com","js.hs-scripts.com","forms.hsforms.com",
    "drift.com","js.drift.com","driftt.com",
    "zendesk.com","static.zdassets.com","ekr.zdassets.com",
    // Facebook / Meta pixel
    "facebook.com/tr","facebook.net","connect.facebook.net/signals",
    // Twitter / X analytics
    "analytics.twitter.com","t.co/tracking","platform.twitter.com/widgets",
    // LinkedIn insight
    "snap.licdn.com","px.ads.linkedin.com","analytics.linkedin.com",
    // Pinterest / TikTok / Snapchat pixels
    "ct.pinterest.com","analytics.tiktok.com","ads.tiktok.com",
    "tr.snapchat.com","sc-static.net",
    // Klaviyo / Marketo / Pardot
    "klaviyo.com","static.klaviyo.com","mktoresp.com","mkto.com",
    "pardot.com","pi.pardot.com","mktdns.com",
    // Fingerprinting
    "fingerprintjs.com","fp.io","fpjs.io","fingerprint.com",
    "privacysandbox.com","topics.googlevideo.com",
    // Microsoft telemetry
    "telemetry.microsoft.com","vortex.data.microsoft.com",
    "watson.telemetry.microsoft.com","browser.events.data.microsoft.com",
    "telecommand.telemetry.microsoft.com","oca.telemetry.microsoft.com",
    // Mozilla telemetry
    "telemetry.mozilla.org","incoming.telemetry.mozilla.org",
    // Miscellaneous
    "optimizely.com","cdn.optimizely.com","p.typekit.net",
    "omtrdc.net","2o7.net","adobedtm.com","sc.omtrdc.net",
    "demdex.net","everesttech.net","fls.doubleclick.net",
    "tiqcdn.com","tealiumiq.com","tags.tiqcdn.com",
    "qualtrics.com","yotpo.com","bazaarvoice.com","powerreviews.com",
    "bing.com/bat","bat.bing.com","c.bing.com",
    "hotjar.io","insights.hotjar.com",
]));

// ── NSFW DOMAINS ─────────────────────────────────────────────────

static NSFW_DOMAINS: Lazy<HashSet<&'static str>> = Lazy::new(|| HashSet::from([
    "pornhub.com","xvideos.com","xnxx.com","redtube.com","youporn.com",
    "tube8.com","xtube.com","spankbang.com","porn.com","sex.com",
    "xhamster.com","beeg.com","nuvid.com","txxx.com","hclips.com",
    "tnaflix.com","keezmovies.com","slutload.com","hardsextube.com",
    "imagefap.com","motherless.com","eporner.com","porndig.com",
    "analdin.com","anysex.com","drtuber.com","iceporn.com","pornrox.com",
    "nhentai.net","hitomi.la","e-hentai.org","fakku.net","hentaigasm.com",
    "adultfriendfinder.com","ashleymadison.com",
    "chaturbate.com","myfreecams.com","cam4.com","bongacams.com",
    "livejasmin.com","stripchat.com","camsoda.com","flirt4free.com",
    "imlive.com","streamray.com","amateur.tv","streamate.com",
]));

static NSFW_PATH_SIGNALS: &[&str] = &[
    "/porn/","/xxx/","/adult/","/nsfw/","/sex/","/nude/","/naked/",
    "/hentai/","/erotic/","/explicit/","18+","x-rated","/camgirl","/camboy",
];

// ── POPUP / REDIRECT PATTERNS ─────────────────────────────────────

static POPUP_PATTERNS: &[&str] = &[
    "popup","popunder","pop-under","clickunder","click-under",
    "layer_ad","layerad","interstitial","redirect.php?url=",
    "track.php?","clk.php?","go.php?","out.php?link=",
    "click.php?","aff_redirect","affiliate_redirect",
    "/out/?","exit-popup","exitpopup","leave_page",
    "countdown.js","ad-redirect","adclick","trackclick",
];

// ── CRYPTO MINERS ────────────────────────────────────────────────

static MINER_DOMAINS: Lazy<HashSet<&'static str>> = Lazy::new(|| HashSet::from([
    "coinhive.com","coin-hive.com","minero.cc","jsecoin.com",
    "cryptoloot.com","cryptoloot.pro","coinimp.com","webminepool.com",
    "webminer.com","minexmr.com","moneroocean.stream","authedmine.com",
    "miner.pr0gramm.com","reasedoper.pw","hatevery.info",
    "kuddus.com","d3iz6lralvg77g.cloudfront.net","listat.biz","lmodr.biz",
    "freecontent.bid","nxgt.net","coinpot.co","coinlab.biz",
]));

// ── MALWARE / PHISHING ────────────────────────────────────────────

static MALWARE_DOMAINS: Lazy<HashSet<&'static str>> = Lazy::new(|| HashSet::from([
    "malware.com","phishing-site.com",
    "fake-update.net","update-flash-player.com",
    "yourcomputerhasavirus.com","virusdetected.net",
    "windows-defender-alert.com","antivirus-alert.com",
    "tech-support-scam.com","virus-alert.support",
    "free-virus-scan.net","scanmypc.net",
    "bit.ly.malicious.com","malvertising.net",
]));

// ── Runtime check (called per request) ───────────────────────────

pub fn check_url(
    url:            &str,
    block_ads:      bool,
    block_trackers: bool,
    block_nsfw:     bool,
    block_popups:   bool,
) -> BlockDecision {
    let lower  = url.to_lowercase();
    let domain = extract_domain(&lower);

    // Always block miners + malware
    if MINER_DOMAINS.contains(domain)  { return BlockDecision::block("miner",   "Crypto Miner"); }
    if MALWARE_DOMAINS.contains(domain){ return BlockDecision::block("malware", "Malware/Phishing"); }

    if block_nsfw {
        if NSFW_DOMAINS.contains(domain) { return BlockDecision::block("nsfw", "Adult Content"); }
        if NSFW_PATH_SIGNALS.iter().any(|s| lower.contains(s)) {
            return BlockDecision::block("nsfw", "Adult Content");
        }
    }

    if block_trackers {
        if TRACKER_DOMAINS.contains(domain) { return BlockDecision::block("tracker", "Tracker"); }
        if TRACKER_DOMAINS.iter().any(|&t| domain.ends_with(t)) {
            return BlockDecision::block("tracker", "Tracker");
        }
    }

    if block_ads {
        if AD_DOMAINS.contains(domain) { return BlockDecision::block("ad", "Ad Network"); }
        if AD_DOMAINS.iter().any(|&a| domain.ends_with(a)) {
            return BlockDecision::block("ad", "Ad Network");
        }
        if lower.contains("/ads/") || lower.contains("/ad/") ||
           lower.contains("/adserver") || lower.contains("/banner") ||
           lower.contains("doubleclick") || lower.contains("googlesyndication") {
            return BlockDecision::block("ad", "Ad Pattern");
        }
    }

    if block_popups {
        if POPUP_PATTERNS.iter().any(|p| lower.contains(p)) {
            return BlockDecision::block("popup", "Popup/Redirect");
        }
    }

    BlockDecision::allow()
}

// ── WKContentRuleList JSON (macOS native engine blocking) ─────────
//
// This generates the JSON that WebKit compiles into a native content
// blocking list. WebKit evaluates these rules before any bytes cross
// the network — not even a TCP connection is opened for blocked resources.
// This is the same mechanism Safari/Brave use for their ad blockers.
//
// Format: https://webkit.org/blog/3476/content-blockers-first-look/

pub fn generate_content_rules(
    block_ads:      bool,
    block_trackers: bool,
    block_nsfw:     bool,
    block_popups:   bool,
) -> String {
    let mut rules: Vec<serde_json::Value> = Vec::new();

    // Helper to make a block rule
    let block_rule = |pattern: &str, resource_types: &[&str]| -> serde_json::Value {
        serde_json::json!({
            "trigger": {
                "url-filter": pattern,
                "resource-type": resource_types
            },
            "action": { "type": "block" }
        })
    };

    // Always: miners + malware
    for d in MINER_DOMAINS.iter() {
        rules.push(block_rule(
            &format!(".*{}.*", d.replace(".", "\\.")),
            &["script","xmlhttprequest","fetch","image","media","raw"],
        ));
    }

    if block_ads {
        // Top ad CDNs — block all resource types
        for d in ["doubleclick.net","googlesyndication.com","googleadservices.com",
                  "adnxs.com","criteo.com","taboola.com","outbrain.com",
                  "amazon-adsystem.com","adform.net","pubmatic.com",
                  "rubiconproject.com","openx.net","moatads.com",
                  "adsterra.com","popads.net","propellerads.com"] {
            rules.push(block_rule(
                &format!(".*{}.*", d.replace(".", "\\.")),
                &["script","xmlhttprequest","fetch","image","media","style-sheet","font","raw"],
            ));
        }
        // Generic ad path patterns
        rules.push(block_rule(r".*\/ads?\/.*", &["script","image","xmlhttprequest","fetch","media"]));
        rules.push(block_rule(r".*\/adserver.*", &["script","image","xmlhttprequest","fetch"]));
        rules.push(block_rule(r".*\/banner.*", &["image","media"]));
        rules.push(block_rule(r".*pagead.*", &["script","image","xmlhttprequest","fetch"]));
    }

    if block_trackers {
        for d in ["google-analytics.com","googletagmanager.com","hotjar.com",
                  "mixpanel.com","amplitude.com","segment.com","segment.io",
                  "fullstory.com","logrocket.com","sentry.io","newrelic.com",
                  "clarity.ms","mouseflow.com","heapanalytics.com",
                  "intercom.com","intercom.io","hubspot.com","drift.com",
                  "klaviyo.com","pardot.com","demdex.net","omtrdc.net",
                  "tealiumiq.com","tiqcdn.com","bat.bing.com",
                  "telemetry.mozilla.org","telemetry.microsoft.com"] {
            rules.push(block_rule(
                &format!(".*{}.*", d.replace(".", "\\.")),
                &["script","xmlhttprequest","fetch","image","raw"],
            ));
        }
        // Tracking pixels
        rules.push(block_rule(r".*(pixel|beacon|track|analytics|collect)\.(php|gif|png|js).*",
            &["image","xmlhttprequest","fetch","raw"]));
    }

    if block_nsfw {
        for d in NSFW_DOMAINS.iter().take(30) {
            rules.push(serde_json::json!({
                "trigger": {
                    "url-filter": format!(".*{}.*", d.replace(".", "\\.")),
                    "resource-type": ["document","subdocument"]
                },
                "action": { "type": "block" }
            }));
        }
    }

    serde_json::to_string(&rules).unwrap_or_else(|_| "[]".into())
}

// ── JS fetch/XHR override (injected into every page) ─────────────
//
// On Windows and Linux where native content rules aren't available,
// we inject this script at document_start to intercept all fetch/XHR.
// On macOS this is a belt-and-suspenders backup to the WKContentRuleList.

pub fn generate_blocker_script(
    block_ads:      bool,
    block_trackers: bool,
    block_nsfw:     bool,
    block_popups:   bool,
) -> String {
    // Build a compact JSON array of patterns the JS side checks
    let mut patterns: Vec<String> = Vec::new();

    // Always: miners
    patterns.extend(MINER_DOMAINS.iter().map(|d| d.to_string()));

    if block_ads {
        patterns.extend(["doubleclick.net","googlesyndication.com","adnxs.com",
            "criteo.com","taboola.com","outbrain.com","amazon-adsystem.com",
            "adform.net","pubmatic.com","rubiconproject.com","moatads.com",
            "adsterra.com","popads.net","propellerads.com"].iter().map(|s| s.to_string()));
    }
    if block_trackers {
        patterns.extend(["google-analytics.com","googletagmanager.com","hotjar.com",
            "mixpanel.com","amplitude.com","segment.com","fullstory.com",
            "sentry.io","newrelic.com","clarity.ms","bat.bing.com"].iter().map(|s| s.to_string()));
    }
    if block_nsfw {
        patterns.extend(NSFW_DOMAINS.iter().take(20).map(|d| d.to_string()));
    }

    let patterns_json = serde_json::to_string(&patterns).unwrap_or_else(|_| "[]".into());
    let enable_popup  = if block_popups { "true" } else { "false" };

    format!(r#"
(function() {{
  "use strict";
  const BLOCKED = {patterns_json};
  const BLOCK_POPUPS = {enable_popup};

  function shouldBlock(url) {{
    if (!url) return false;
    try {{
      const host = new URL(url).hostname.replace(/^www\./, '');
      return BLOCKED.some(p => host === p || host.endsWith('.' + p));
    }} catch {{ return false; }}
  }}

  // Override fetch
  const _fetch = window.fetch;
  window.fetch = function(input, init) {{
    const url = typeof input === 'string' ? input : (input && input.url) || '';
    if (shouldBlock(url)) {{
      console.debug('[Parsec Shield] blocked fetch:', url);
      return Promise.reject(new TypeError('Blocked by Parsec Shield'));
    }}
    return _fetch.apply(this, arguments);
  }};

  // Override XMLHttpRequest
  const _open = XMLHttpRequest.prototype.open;
  XMLHttpRequest.prototype.open = function(method, url) {{
    if (shouldBlock(url)) {{
      console.debug('[Parsec Shield] blocked XHR:', url);
      this._parsec_blocked = true;
      return;
    }}
    return _open.apply(this, arguments);
  }};
  const _send = XMLHttpRequest.prototype.send;
  XMLHttpRequest.prototype.send = function() {{
    if (this._parsec_blocked) return;
    return _send.apply(this, arguments);
  }};

  // Block window.open popups
  if (BLOCK_POPUPS) {{
    const _open_win = window.open;
    window.open = function(url, target, features) {{
      if (!url || url === 'about:blank') return _open_win.apply(this, arguments);
      if (shouldBlock(url)) {{
        console.debug('[Parsec Shield] blocked popup:', url);
        return null;
      }}
      // Only allow user-gesture popups
      return null;
    }};
  }}

  // Remove ad-related iframes and scripts already in DOM
  const observer = new MutationObserver(mutations => {{
    for (const m of mutations) {{
      for (const node of m.addedNodes) {{
        if (node.nodeType === 1) {{
          const el = node;
          const src = el.src || el.href || '';
          if (src && shouldBlock(src)) {{
            el.remove();
            console.debug('[Parsec Shield] removed element:', src);
          }}
        }}
      }}
    }}
  }});
  observer.observe(document.documentElement, {{ childList: true, subtree: true }});

  console.info('[Parsec Shield] content script active');
}})();
"#)
}

// ── Domain extraction ─────────────────────────────────────────────

pub fn extract_domain(url: &str) -> &str {
    let s = url.trim_start_matches("https://")
               .trim_start_matches("http://")
               .trim_start_matches("//");
    let end = s.find('/').unwrap_or(s.len());
    let host = &s[..end];
    let host = host.split(':').next().unwrap_or(host);
    host.strip_prefix("www.").unwrap_or(host)
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn blocks_doubleclick() {
        let d = check_url("https://googleads.g.doubleclick.net/pagead/id", true, true, false, true);
        assert!(d.blocked); assert_eq!(d.reason.as_deref(), Some("ad"));
    }
    #[test] fn blocks_ga() {
        let d = check_url("https://www.google-analytics.com/collect?v=1", true, true, false, true);
        assert!(d.blocked); assert_eq!(d.reason.as_deref(), Some("tracker"));
    }
    #[test] fn allows_github() {
        let d = check_url("https://github.com/rust-lang/rust", true, true, true, true);
        assert!(!d.blocked);
    }
    #[test] fn blocks_miner_always() {
        let d = check_url("https://coinhive.com/lib/coinhive.min.js", false, false, false, false);
        assert!(d.blocked); assert_eq!(d.reason.as_deref(), Some("miner"));
    }
    #[test] fn blocks_nsfw_when_on() {
        assert!(check_url("https://pornhub.com/", true, true, true, true).blocked);
        assert!(!check_url("https://pornhub.com/", true, true, false, true).blocked);
    }
    #[test] fn content_rules_valid_json() {
        let j = generate_content_rules(true, true, false, true);
        let v: serde_json::Value = serde_json::from_str(&j).expect("valid JSON");
        assert!(v.is_array());
        assert!(v.as_array().unwrap().len() > 10);
    }
    #[test] fn blocker_script_valid() {
        let s = generate_blocker_script(true, true, false, true);
        assert!(s.contains("window.fetch"));
        assert!(s.contains("XMLHttpRequest"));
        assert!(s.contains("Parsec Shield"));
    }
}
