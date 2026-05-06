// @ts-nocheck
// ================================================================
//  ParsecWeb.tsx v1.3
//  Real Chrome Web Store search (live API)
//  Profile persistence · Tab suspension · Session restore
//  Full IPC to wry per-tab WebViews · HTTP/3 · Neutron GPU
// ================================================================

import {
  useState, useEffect, useRef, useCallback,
  KeyboardEvent,
} from "react";

// ─── IPC BRIDGE ──────────────────────────────────────────────────

let _msgId = 0;
const _pending = new Map<string, (v: unknown) => void>();

// Receive responses from Rust
if (typeof window !== "undefined") {
  (window as any).__parsec_reply = (resp: { id: string; ok: boolean; data: unknown; error?: string }) => {
    const cb = _pending.get(resp.id);
    if (cb) { _pending.delete(resp.id); cb(resp.data); }
  };
  (window as any).__parsec_chrome_event = handleRustEvent;
}

function ipc<T>(cmd: string, args: object = {}): Promise<T> {
  return new Promise(resolve => {
    const id = String(++_msgId);
    _pending.set(id, v => resolve(v as T));
    const msg = JSON.stringify({ id, cmd, args });
    if ((window as any).ipc) (window as any).ipc.postMessage(msg);
    else setTimeout(() => resolve(getMock(cmd, args) as T), 30);
  });
}

// Mocks for browser-based development
function getMock(cmd: string, args: any): unknown {
  switch (cmd) {
    case "GetPrivacyStats": return { ads_blocked: 2841, trackers_blocked: 482, popups_blocked: 17, nsfw_blocked: 0, miners_blocked: 3, bytes_saved: 6_200_000, requests_total: 14230, session_start: Date.now() - 3600000 };
    case "GetPrefs":        return { block_ads: true, block_trackers: true, block_nsfw: false, block_popups: true, https_only: true, do_not_track: true, prefetch: true, auto_suspend_tabs: true, suspend_after_secs: 300, default_engine: "Parsec Search" };
    case "GetBookmarks":    return DEF_BOOKMARKS;
    case "GetHistory":      return DEF_HISTORY;
    case "GetSuggestions":  return [{ query: args.query, url: `https://search.parsec.os/search?q=${encodeURIComponent(args.query)}` }];
    case "GetCertInfo":     return args.url?.startsWith("https") ? { subject: `CN=${tryHost(args.url)}`, issuer: "Let's Encrypt", valid_until: "2027-02-15", fingerprint: "A1:B2:C3:D4:E5:F6", san_domains: [tryHost(args.url)], is_ev: false, is_trusted: true, protocol: "HTTP/3", cipher: "TLS_AES_256_GCM_SHA384" } : null;
    case "CwsSearch":       return { extensions: MOCK_CWS_RESULTS.filter(e => !args.query || e.name.toLowerCase().includes(args.query.toLowerCase())), total: MOCK_CWS_RESULTS.length, page: 0, has_more: false };
    case "CwsFeatured":     return { extensions: MOCK_CWS_RESULTS.filter(e => args.category === "All" || e.category === args.category), total: MOCK_CWS_RESULTS.length, page: 0, has_more: false };
    case "CwsListInstalled":return DEF_EXTS.map(e => ({ ...e, mv: 3 }));
    case "GetSessions":     return [];
    default:                return {};
  }
}
function tryHost(url: string) { try { return new URL(url).hostname; } catch { return "unknown"; } }

// ─── RUST EVENTS ─────────────────────────────────────────────────

type RustEvent = { type: string; tabId?: string; [k: string]: unknown };
type TabUpdater = (id: string, fn: (t: Tab) => Tab) => void;

let _updateTab: TabUpdater | null = null;
let _setCertInfo: ((c: CertInfo | null) => void) | null = null;
let _setInstallProgress: ((p: InstallProgress) => void) | null = null;

function handleRustEvent(ev: RustEvent) {
  const { type: t, tabId } = ev;
  if (tabId && _updateTab) {
    switch (t) {
      case "TitleChanged":   _updateTab(tabId, tab => ({ ...tab, title: ev.title as string })); break;
      case "UrlChanged":     _updateTab(tabId, tab => ({ ...tab, url: ev.url as string, loading: true })); break;
      case "LoadStart":      _updateTab(tabId, tab => ({ ...tab, loading: true, blocked: false })); break;
      case "LoadFinish":     _updateTab(tabId, tab => ({ ...tab, loading: false })); break;
      case "Blocked":        _updateTab(tabId, tab => ({ ...tab, blocked: true, loading: false, blockReason: ev.reason as string })); break;
      case "CanNavigate":    _updateTab(tabId, tab => ({ ...tab, canGoBack: ev.can_back as boolean, canGoFwd: ev.can_fwd as boolean })); break;
      case "FaviconChanged": _updateTab(tabId, tab => ({ ...tab, favicon: ev.favicon_url as string })); break;
      case "Suspended":      _updateTab(tabId, tab => ({ ...tab, suspended: true })); break;
    }
  }
  if (t === "CertInfo")         _setCertInfo?.(ev.cert as CertInfo);
  if (t === "InstallProgress")  _setInstallProgress?.(ev as InstallProgress);
}

// ─── TYPES ───────────────────────────────────────────────────────

interface Tab {
  id: string; url: string; title: string; favicon: string;
  loading: boolean; canGoBack: boolean; canGoFwd: boolean;
  pinned: boolean; muted: boolean; audible: boolean;
  incognito: boolean; zoom: number;
  blocked: boolean; blockReason?: string; suspended: boolean;
}

interface CwsExtension {
  id: string; name: string; author: string; version: string;
  description: string; icon_url: string | null; rating: number;
  rating_count: number; user_count: string; category: string;
  featured: boolean; price: string; last_updated: string;
  store_url: string;
}

interface InstalledExtension {
  id: string; name: string; version: string; description: string;
  icon: string; iconBg: string; enabled: boolean;
  mv: 2 | 3; permissions: string[];
}

interface InstallProgress {
  type?: string; ext_id: string; stage: string; percent: number; message: string;
}

interface CertInfo {
  subject: string; issuer: string; valid_until: string;
  fingerprint: string; san_domains: string[];
  is_ev: boolean; is_trusted: boolean; protocol: string; cipher: string;
}

interface PrivacyStats {
  ads_blocked: number; trackers_blocked: number;
  popups_blocked: number; nsfw_blocked: number; miners_blocked: number;
  bytes_saved: number; requests_total: number;
}

interface HistoryItem { id: string; url: string; title: string; visit_time: number; favicon: string; visit_count?: number; }
interface BookmarkItem { id: string; url: string; title: string; favicon: string; folder?: string; }
interface Suggestion   { type: "history"|"bookmark"|"search"; url: string; title: string; favicon?: string; }
interface DownloadItem { id: string; filename: string; url: string; progress: number; size: number; downloaded: number; state: string; speed_bps: number; }
interface TabSession   { id: string; label: string; saved_at: number; tabs: { url: string; title: string }[]; }

type Panel = "none"|"history"|"downloads"|"bookmarks"|"extensions"|"settings"|"sessions"|"sync";

// ─── UTILS ───────────────────────────────────────────────────────

const genId  = () => Math.random().toString(36).slice(2, 10);
const fmtB   = (b: number) => b < 1024 ? `${b}B` : b < 1048576 ? `${(b/1024).toFixed(1)}KB` : `${(b/1048576).toFixed(1)}MB`;
const fmtSpd = (b: number) => b < 1024 ? `${b}B/s` : b < 1048576 ? `${(b/1024).toFixed(0)}KB/s` : `${(b/1048576).toFixed(1)}MB/s`;
const ago    = (ts: number) => { const d=(Date.now()-ts)/1000; return d<60?"just now":d<3600?`${~~(d/60)}m ago`:d<86400?`${~~(d/3600)}h ago`:`${~~(d/86400)}d ago`; };
const fmtN   = (n: number) => n>=1e6?`${(n/1e6).toFixed(1)}M`:n>=1000?`${(n/1000).toFixed(0)}K`:`${n}`;

function truncUrl(url: string, max = 60) {
  try { const u=new URL(url); const d=u.hostname+u.pathname.replace(/\/$/,""); return d.length>max?d.slice(0,max)+"…":d; }
  catch { return url.length>max?url.slice(0,max)+"…":url; }
}

function normalizeUrl(input: string): string {
  if (/^(parsec:|about:)/.test(input)) return input;
  if (/^https?:\/\//.test(input)) return input;
  if (/^[a-z0-9][-a-z0-9]*\.[a-z]{2,}/i.test(input) && !input.includes(' ')) return `https://${input}`;
  return `https://search.parsec.os/search?q=${encodeURIComponent(input)}`;
}

// ─── DEFAULTS ────────────────────────────────────────────────────

const DEF_EXTS: InstalledExtension[] = [
  { id:"parsec-shield",    name:"Parsec Shield",    version:"1.3.0", description:"Real 3-layer blocking: WKContentRuleList + fetch/XHR override + navigation handler. 500+ ad domains, 200+ trackers, NSFW, miners.", icon:"🛡️", iconBg:"linear-gradient(135deg,#c53030,#e53e3e)", enabled:true, mv:3, permissions:["declarativeNetRequest","webRequest","tabs"] },
  { id:"parsec-passwords", name:"Parsec Passwords", version:"1.1.0", description:"E2E encrypted vault. Autofill on any site. Biometric unlock.",                                                                         icon:"🔐", iconBg:"linear-gradient(135deg,#1a73e8,#175db8)", enabled:true, mv:3, permissions:["storage","tabs","scripting"] },
  { id:"parsec-reader",    name:"Parsec Reader",    version:"1.5.0", description:"Distraction-free reading mode with TTS and offline save.",                                                                             icon:"📖", iconBg:"linear-gradient(135deg,#2d7dd2,#48bb78)", enabled:true, mv:3, permissions:["tabs","scripting"] },
];

// Mock CWS results (shown when offline / API unavailable)
const MOCK_CWS_RESULTS: CwsExtension[] = [
  { id:"cjpalhdlnbpafiamejdnhcphjbkeiagm", name:"uBlock Origin", author:"Raymond Hill", version:"1.57.2", description:"Finally, an efficient blocker. Easy on CPU and memory. Block ads, trackers, malware, and more with thousands of filters.", icon_url:"https://lh3.googleusercontent.com/vRQEUBGNx6IGGK0pLYQWe_n6ICeq1SBFHKwg02Mk6nXm0aNvRY0dSQQ7Qj5lQ7T07fhRSSv", rating:4.9, rating_count:374891, user_count:"10,000,000+", category:"Privacy", featured:true, price:"Free", last_updated:"Feb 2026", store_url:"https://chrome.google.com/webstore/detail/cjpalhdlnbpafiamejdnhcphjbkeiagm" },
  { id:"eimadpbcbfnmbkopoojfekhnkhdbieeh", name:"Dark Reader", author:"Alexander Shutau", version:"4.9.88", description:"Dark mode for every website. Take care of your eyes, use dark theme for night and daily browsing.", icon_url:null, rating:4.8, rating_count:98241, user_count:"5,000,000+", category:"Accessibility", featured:true, price:"Free", last_updated:"Jan 2026", store_url:"https://chrome.google.com/webstore/detail/eimadpbcbfnmbkopoojfekhnkhdbieeh" },
  { id:"nngceckbapebfimnlniiiahkandclblb", name:"Bitwarden", author:"Bitwarden Inc.", version:"2026.2.0", description:"Open source password manager. Secure, cross-platform password storage with autofill.", icon_url:null, rating:4.8, rating_count:52190, user_count:"2,000,000+", category:"Productivity", featured:true, price:"Free", last_updated:"Feb 2026", store_url:"https://chrome.google.com/webstore/detail/nngceckbapebfimnlniiiahkandclblb" },
  { id:"kbfnbcaeplbcioakkpcpgfkobkghlhen", name:"Grammarly", author:"Grammarly, Inc.", version:"14.1115.0", description:"Spell check, grammar corrections, and writing suggestions everywhere you type on the web.", icon_url:null, rating:4.6, rating_count:183422, user_count:"10,000,000+", category:"Productivity", featured:false, price:"Free", last_updated:"Feb 2026", store_url:"https://chrome.google.com/webstore/detail/kbfnbcaeplbcioakkpcpgfkobkghlhen" },
  { id:"hgmhmanpeganjkgimnchfkndapdcecie", name:"Wappalyzer", author:"Wappalyzer", version:"6.10.74", description:"Uncover the CMS, ecommerce platform, JavaScript frameworks, analytics tools and more on any website.", icon_url:null, rating:4.4, rating_count:23817, user_count:"1,000,000+", category:"Dev Tools", featured:false, price:"Free", last_updated:"Dec 2025", store_url:"https://chrome.google.com/webstore/detail/hgmhmanpeganjkgimnchfkndapdcecie" },
  { id:"bcjindcccaagfpapjjmafapmmgkkhgoa", name:"JSON Formatter", author:"Callum Locke", version:"0.6.0", description:"Makes JSON easy to read. Open a JSON file or URL and this will auto-format it.", icon_url:null, rating:4.7, rating_count:18432, user_count:"500,000+", category:"Dev Tools", featured:false, price:"Free", last_updated:"Nov 2025", store_url:"https://chrome.google.com/webstore/detail/bcjindcccaagfpapjjmafapmmgkkhgoa" },
  { id:"dhdgffkkebhmkfjojejmpbldmpobfkfo", name:"Tampermonkey", author:"Jan Biniok", version:"5.1.0", description:"The most popular userscript manager with over 10 million users. Run scripts to customize websites.", icon_url:null, rating:4.7, rating_count:61823, user_count:"10,000,000+", category:"Productivity", featured:false, price:"Free", last_updated:"Jan 2026", store_url:"https://chrome.google.com/webstore/detail/dhdgffkkebhmkfjojejmpbldmpobfkfo" },
  { id:"bmnlcjabgnpnenekpadlanbbkooimhnj", name:"Honey", author:"PayPal", version:"16.4.3", description:"Automatically finds and applies coupon codes when you shop online. Saves money effortlessly.", icon_url:null, rating:4.5, rating_count:234182, user_count:"10,000,000+", category:"Shopping", featured:false, price:"Free", last_updated:"Jan 2026", store_url:"https://chrome.google.com/webstore/detail/bmnlcjabgnpnenekpadlanbbkooimhnj" },
  { id:"blipmdconlkpinefehnmjammfjpmpbjk", name:"Lighthouse", author:"Google", version:"11.1.0", description:"Automated tool for improving performance, accessibility, SEO of web pages.", icon_url:null, rating:4.6, rating_count:12309, user_count:"1,000,000+", category:"Dev Tools", featured:false, price:"Free", last_updated:"Feb 2026", store_url:"https://chrome.google.com/webstore/detail/blipmdconlkpinefehnmjammfjpmpbjk" },
  { id:"ohbcdjpcgpnmockpimflkagfcoelmacc", name:"ColorZilla", author:"Alex Sirota", version:"3.3", description:"Advanced eyedropper, color picker, gradient generator and CSS color tools.", icon_url:null, rating:4.5, rating_count:8934, user_count:"2,000,000+", category:"Dev Tools", featured:false, price:"Free", last_updated:"Oct 2025", store_url:"https://chrome.google.com/webstore/detail/ohbcdjpcgpnmockpimflkagfcoelmacc" },
  { id:"aapbdbdomjkkjkaonfhkkikfgjllcleb", name:"Google Translate", author:"Google", version:"2.0.13", description:"View translations easily as you browse the web. Translate pages automatically.", icon_url:null, rating:4.2, rating_count:289041, user_count:"10,000,000+", category:"Productivity", featured:false, price:"Free", last_updated:"Jan 2026", store_url:"https://chrome.google.com/webstore/detail/aapbdbdomjkkjkaonfhkkikfgjllcleb" },
  { id:"mnjggcdmjocbbbhaepdhchncahnbgone", name:"SponsorBlock", author:"Ajay Ramachandran", version:"5.7.3", description:"Skip sponsorships, intros, outros, and other annoying parts in YouTube videos.", icon_url:null, rating:4.9, rating_count:41203, user_count:"500,000+", category:"Fun", featured:false, price:"Free", last_updated:"Feb 2026", store_url:"https://chrome.google.com/webstore/detail/mnjggcdmjocbbbhaepdhchncahnbgone" },
];

const DEF_BOOKMARKS: BookmarkItem[] = [
  { id:genId(), url:"https://github.com",            title:"GitHub",       favicon:"🐙", folder:"Dev"  },
  { id:genId(), url:"https://news.ycombinator.com",  title:"Hacker News",  favicon:"🟧", folder:"Dev"  },
  { id:genId(), url:"https://claude.ai",             title:"Claude AI",    favicon:"🤖", folder:"AI"   },
  { id:genId(), url:"https://linear.app",            title:"Linear",       favicon:"📋", folder:"Work" },
  { id:genId(), url:"https://developer.mozilla.org", title:"MDN",          favicon:"📚", folder:"Dev"  },
];

const DEF_HISTORY: HistoryItem[] = [
  { id:genId(), url:"https://github.com",           title:"GitHub",       visit_time:Date.now()-300000,  favicon:"🐙", visit_count:12 },
  { id:genId(), url:"https://news.ycombinator.com", title:"Hacker News",  visit_time:Date.now()-600000,  favicon:"🟧", visit_count:5  },
  { id:genId(), url:"https://claude.ai",            title:"Claude AI",    visit_time:Date.now()-900000,  favicon:"🤖", visit_count:23 },
];

// ─── ICONS ───────────────────────────────────────────────────────

const IC = {
  Back:   ()=><svg viewBox="0 0 20 20" fill="currentColor" width="16" height="16"><path fillRule="evenodd" d="M9.707 16.707a1 1 0 01-1.414 0l-6-6a1 1 0 010-1.414l6-6a1 1 0 011.414 1.414L5.414 9H17a1 1 0 110 2H5.414l4.293 4.293a1 1 0 010 1.414z" clipRule="evenodd"/></svg>,
  Fwd:    ()=><svg viewBox="0 0 20 20" fill="currentColor" width="16" height="16"><path fillRule="evenodd" d="M10.293 3.293a1 1 0 011.414 0l6 6a1 1 0 010 1.414l-6 6a1 1 0 01-1.414-1.414L14.586 11H3a1 1 0 110-2h11.586l-4.293-4.293a1 1 0 010-1.414z" clipRule="evenodd"/></svg>,
  Reload: ()=><svg viewBox="0 0 20 20" fill="currentColor" width="16" height="16"><path fillRule="evenodd" d="M4 2a1 1 0 011 1v2.101a7.002 7.002 0 0111.601 2.566 1 1 0 11-1.885.666A5.002 5.002 0 005.999 7H9a1 1 0 010 2H4a1 1 0 01-1-1V3a1 1 0 011-1zm.008 9.057a1 1 0 011.276.61A5.002 5.002 0 0014.001 13H11a1 1 0 110-2h5a1 1 0 011 1v5a1 1 0 11-2 0v-2.101a7.002 7.002 0 01-11.601-2.566 1 1 0 01.61-1.276z" clipRule="evenodd"/></svg>,
  Home:   ()=><svg viewBox="0 0 20 20" fill="currentColor" width="16" height="16"><path d="M10.707 2.293a1 1 0 00-1.414 0l-7 7a1 1 0 001.414 1.414L4 10.414V17a1 1 0 001 1h2a1 1 0 001-1v-2a1 1 0 011-1h2a1 1 0 011 1v2a1 1 0 001 1h2a1 1 0 001-1v-6.586l.293.293a1 1 0 001.414-1.414l-7-7z"/></svg>,
  Plus:   ()=><svg viewBox="0 0 20 20" fill="currentColor" width="14" height="14"><path fillRule="evenodd" d="M10 3a1 1 0 011 1v5h5a1 1 0 110 2h-5v5a1 1 0 11-2 0v-5H4a1 1 0 110-2h5V4a1 1 0 011-1z" clipRule="evenodd"/></svg>,
  X:      ()=><svg viewBox="0 0 20 20" fill="currentColor" width="12" height="12"><path fillRule="evenodd" d="M4.293 4.293a1 1 0 011.414 0L10 8.586l4.293-4.293a1 1 0 111.414 1.414L11.414 10l4.293 4.293a1 1 0 01-1.414 1.414L10 11.414l-4.293 4.293a1 1 0 01-1.414-1.414L8.586 10 4.293 5.707a1 1 0 010-1.414z" clipRule="evenodd"/></svg>,
  Star:   ({f}:{f?:boolean})=><svg viewBox="0 0 20 20" fill={f?"currentColor":"none"} stroke="currentColor" strokeWidth={f?0:1.5} width="16" height="16"><path strokeLinecap="round" strokeLinejoin="round" d="M9.049 2.927c.3-.921 1.603-.921 1.902 0l1.07 3.292a1 1 0 00.95.69h3.462c.969 0 1.371 1.24.588 1.81l-2.8 2.034a1 1 0 00-.364 1.118l1.07 3.292c.3.921-.755 1.688-1.54 1.118l-2.8-2.034a1 1 0 00-1.175 0l-2.8 2.034c-.784.57-1.838-.197-1.539-1.118l1.07-3.292a1 1 0 00-.364-1.118L2.98 8.72c-.783-.57-.38-1.81.588-1.81h3.461a1 1 0 00.951-.69l1.07-3.292z"/></svg>,
  Menu:   ()=><svg viewBox="0 0 20 20" fill="currentColor" width="16" height="16"><path fillRule="evenodd" d="M3 5a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1zM3 10a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1zM3 15a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1z" clipRule="evenodd"/></svg>,
  Srch:   ()=><svg viewBox="0 0 20 20" fill="currentColor" width="14" height="14"><path fillRule="evenodd" d="M8 4a4 4 0 100 8 4 4 0 000-8zM2 8a6 6 0 1110.89 3.476l4.817 4.817a1 1 0 01-1.414 1.414l-4.816-4.816A6 6 0 012 8z" clipRule="evenodd"/></svg>,
  Check:  ()=><svg viewBox="0 0 20 20" fill="currentColor" width="12" height="12"><path fillRule="evenodd" d="M16.707 5.293a1 1 0 010 1.414l-8 8a1 1 0 01-1.414 0l-4-4a1 1 0 011.414-1.414L8 12.586l7.293-7.293a1 1 0 011.414 0z" clipRule="evenodd"/></svg>,
  Lock:   ({ok=true}:{ok?:boolean})=><svg viewBox="0 0 20 20" fill="currentColor" width="12" height="12" style={{color:ok?"var(--ok)":"var(--danger)"}}><path fillRule="evenodd" d={ok?"M5 9V7a5 5 0 0110 0v2a2 2 0 012 2v5a2 2 0 01-2 2H5a2 2 0 01-2-2v-5a2 2 0 012-2zm8-2v2H7V7a3 3 0 016 0z":"M18 8a6 6 0 01-7.743 5.743L10 14l-1 1-1 1H6v2H2v-4l4.257-4.257A6 6 0 1118 8zm-6-4a1 1 0 100 2 2 2 0 012 2 1 1 0 102 0 4 4 0 00-4-4z"} clipRule="evenodd"/></svg>,
  Shield: ()=><svg viewBox="0 0 20 20" fill="currentColor" width="11" height="11"><path fillRule="evenodd" d="M2.166 4.999A11.954 11.954 0 0010 1.944 11.954 11.954 0 0017.834 5c.11.65.166 1.32.166 2.001 0 5.225-3.34 9.67-8 11.317C5.34 16.67 2 12.225 2 7c0-.682.057-1.35.166-2.001zm11.541 3.708a1 1 0 00-1.414-1.414L9 10.586 7.707 9.293a1 1 0 00-1.414 1.414l2 2a1 1 0 001.414 0l4-4z" clipRule="evenodd"/></svg>,
  Spin:   ()=><div style={{width:12,height:12,borderRadius:"50%",border:"1.5px solid var(--accent)",borderTopColor:"transparent",animation:"spin .6s linear infinite"}}/>,
  Sleep:  ()=><svg viewBox="0 0 20 20" fill="currentColor" width="11" height="11"><path d="M17.293 13.293A8 8 0 016.707 2.707a8.001 8.001 0 1010.586 10.586z"/></svg>,
  Session:()=><svg viewBox="0 0 20 20" fill="currentColor" width="14" height="14"><path d="M5 3a2 2 0 00-2 2v2a2 2 0 002 2h2a2 2 0 002-2V5a2 2 0 00-2-2H5zM5 11a2 2 0 00-2 2v2a2 2 0 002 2h2a2 2 0 002-2v-2a2 2 0 00-2-2H5zM11 5a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2h-2a2 2 0 01-2-2V5zM14 11a1 1 0 011 1v1h1a1 1 0 110 2h-1v1a1 1 0 11-2 0v-1h-1a1 1 0 110-2h1v-1a1 1 0 011-1z"/></svg>,
  Extern: ()=><svg viewBox="0 0 20 20" fill="currentColor" width="11" height="11"><path d="M11 3a1 1 0 100 2h2.586l-6.293 6.293a1 1 0 101.414 1.414L15 6.414V9a1 1 0 102 0V4a1 1 0 00-1-1h-5z"/><path d="M5 5a2 2 0 00-2 2v8a2 2 0 002 2h8a2 2 0 002-2v-3a1 1 0 10-2 0v3H5V7h3a1 1 0 000-2H5z"/></svg>,
};

// ─── STAR RATING ─────────────────────────────────────────────────

function Stars({r,n}:{r:number;n:number}) {
  return (
    <div style={{display:"flex",alignItems:"center",gap:4}}>
      {Array.from({length:5},(_,i)=>(
        <svg key={i} viewBox="0 0 16 16" width="11" height="11">
          <polygon points="8,1 10.2,6 15.5,6.5 11.5,10 12.9,15.2 8,12.3 3.1,15.2 4.5,10 0.5,6.5 5.8,6"
            fill={i<Math.floor(r)?"#ecc94b":i<r?"url(#hg)":"#4a5568"}/>
          {i<r&&i>=Math.floor(r)&&<defs><linearGradient id="hg"><stop offset="50%" stopColor="#ecc94b"/><stop offset="50%" stopColor="#4a5568"/></linearGradient></defs>}
        </svg>
      ))}
      <span style={{fontSize:10,color:"var(--muted)"}}>{r.toFixed(1)} · {fmtN(n)}</span>
    </div>
  );
}

// ─── CHROME WEB STORE PANEL ──────────────────────────────────────
//
// Hits the real CWS API from Rust backend.
// Shows live search results, real ratings, real install counts.
// Downloads + installs real .crx files.

const CWS_CATS = ["All","Featured","Productivity","Privacy","Accessibility","Dev Tools","Shopping","Fun"];

function ChromeWebStore({
  installed, onInstall, onUninstall,
}: {
  installed: Set<string>;
  onInstall: (id: string) => void;
  onUninstall: (id: string) => void;
}) {
  const [q, setQ]               = useState("");
  const [cat, setCat]           = useState("Featured");
  const [results, setResults]   = useState<CwsExtension[]>([]);
  const [loading, setLoading]   = useState(false);
  const [page, setPage]         = useState(0);
  const [hasMore, setHasMore]   = useState(false);
  const [installProgress, setInstallProgress] = useState<Map<string, InstallProgress>>(new Map());
  const searchRef               = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Load featured on mount
  useEffect(() => { loadFeatured("Featured"); }, []);

  // Wire install progress events
  useEffect(() => {
    _setInstallProgress = (p: InstallProgress) => {
      setInstallProgress(prev => new Map(prev).set(p.ext_id, p));
      if (p.stage === "done") {
        setTimeout(() => setInstallProgress(prev => { const m = new Map(prev); m.delete(p.ext_id); return m; }), 2000);
      }
    };
    return () => { _setInstallProgress = null; };
  }, []);

  const loadFeatured = async (category: string) => {
    setLoading(true);
    const r = await ipc<{ extensions: CwsExtension[]; has_more: boolean }>("CwsFeatured", { category });
    setResults(r?.extensions || MOCK_CWS_RESULTS);
    setHasMore(r?.has_more || false);
    setLoading(false);
  };

  const doSearch = async (query: string, p = 0) => {
    if (!query.trim()) { loadFeatured(cat); return; }
    setLoading(true);
    const r = await ipc<{ extensions: CwsExtension[]; has_more: boolean }>("CwsSearch", { query, page: p });
    if (p === 0) setResults(r?.extensions || []);
    else setResults(prev => [...prev, ...(r?.extensions || [])]);
    setHasMore(r?.has_more || false);
    setPage(p);
    setLoading(false);
  };

  const handleSearch = (v: string) => {
    setQ(v);
    if (searchRef.current) clearTimeout(searchRef.current);
    searchRef.current = setTimeout(() => doSearch(v, 0), 400);
  };

  const handleCat = (c: string) => {
    setCat(c); setQ(""); setPage(0);
    loadFeatured(c);
  };

  const handleInstall = (ext: CwsExtension) => {
    onInstall(ext.id);
    ipc("CwsInstall", { ext_id: ext.id });
  };

  const extIcon = (ext: CwsExtension) => {
    if (ext.icon_url) return <img src={ext.icon_url} style={{width:44,height:44,borderRadius:11,objectFit:"cover"}} onError={e=>{(e.target as HTMLImageElement).style.display="none";}}/>;
    // Deterministic gradient fallback
    const h = ext.id.charCodeAt(0) * 7 + ext.id.charCodeAt(1) * 13;
    const hue = h % 360;
    return (
      <div style={{width:44,height:44,borderRadius:11,flexShrink:0,
        background:`linear-gradient(135deg,hsl(${hue},60%,40%),hsl(${(hue+40)%360},60%,50%))`,
        display:"flex",alignItems:"center",justifyContent:"center",fontSize:20,color:"white"}}>
        🧩
      </div>
    );
  };

  return (
    <div style={{display:"flex",flexDirection:"column",height:"100%",overflow:"hidden"}}>
      {/* Header */}
      <div style={{padding:"10px 14px 0",flexShrink:0}}>
        <div style={{fontSize:11,fontWeight:700,color:"var(--fg)",marginBottom:8,display:"flex",alignItems:"center",gap:6}}>
          <span style={{fontSize:14}}>🧩</span> Chrome Web Store
          <span style={{fontSize:9,color:"var(--muted)",marginLeft:"auto"}}>Live · 200,000+ extensions</span>
        </div>
        {/* Search */}
        <div style={{display:"flex",alignItems:"center",gap:7,background:"var(--bg2)",border:"1px solid var(--border)",borderRadius:8,padding:"6px 10px",marginBottom:8}}>
          <span style={{color:"var(--muted)",display:"flex"}}><IC.Srch/></span>
          <input value={q} onChange={e=>handleSearch(e.target.value)}
            placeholder="Search 200,000+ Chrome extensions…"
            style={{flex:1,background:"transparent",border:"none",outline:"none",fontSize:12,color:"var(--fg)",fontFamily:"inherit"}}/>
          {loading&&<IC.Spin/>}
          {q&&!loading&&<button onClick={()=>{setQ("");loadFeatured(cat);}} style={{background:"none",border:"none",cursor:"pointer",color:"var(--muted)",display:"flex",padding:0}}><IC.X/></button>}
        </div>
        {/* Category chips */}
        <div style={{display:"flex",gap:5,overflowX:"auto",scrollbarWidth:"none",paddingBottom:8}}>
          {CWS_CATS.map(c=>(
            <button key={c} onClick={()=>handleCat(c)}
              style={{flexShrink:0,padding:"3px 10px",borderRadius:20,fontSize:10,fontFamily:"inherit",
                border:`1px solid ${cat===c&&!q?"var(--accent)":"var(--border)"}`,
                background:cat===c&&!q?"rgba(124,106,255,.15)":"transparent",
                color:cat===c&&!q?"var(--accent)":"var(--muted)",cursor:"pointer",transition:"all .1s"}}>
              {c}
            </button>
          ))}
        </div>
      </div>

      {/* Results */}
      <div style={{flex:1,overflowY:"auto",padding:"0 14px"}}>
        {results.length===0&&!loading&&(
          <div style={{textAlign:"center",padding:"32px 0",color:"var(--muted)",fontSize:12}}>
            {q ? "No results found" : "Loading…"}
          </div>
        )}

        {results.map(ext => {
          const prog = installProgress.get(ext.id);
          const isInstalled = installed.has(ext.id);

          return (
            <div key={ext.id} style={{display:"flex",gap:10,padding:"10px 0",borderBottom:"1px solid var(--border0)",alignItems:"flex-start"}}>
              {/* Icon */}
              <div style={{flexShrink:0,position:"relative"}}>
                {extIcon(ext)}
                {ext.featured&&<div style={{position:"absolute",top:-3,right:-3,background:"#ecc94b",borderRadius:"50%",width:13,height:13,display:"flex",alignItems:"center",justifyContent:"center",fontSize:7}}>⭐</div>}
              </div>

              {/* Info */}
              <div style={{flex:1,minWidth:0}}>
                <div style={{display:"flex",alignItems:"flex-start",justifyContent:"space-between",gap:6,marginBottom:2}}>
                  <div style={{minWidth:0}}>
                    <div style={{fontSize:12,fontWeight:600,color:"var(--fg)",overflow:"hidden",textOverflow:"ellipsis",whiteSpace:"nowrap"}}>{ext.name}</div>
                    <div style={{fontSize:9,color:"var(--muted)"}}>by {ext.author}</div>
                  </div>

                  {/* Install/Uninstall button */}
                  {isInstalled ? (
                    <button onClick={()=>onUninstall(ext.id)}
                      style={{flexShrink:0,padding:"4px 10px",borderRadius:6,fontSize:10,fontFamily:"inherit",cursor:"pointer",border:"1px solid var(--border)",background:"transparent",color:"var(--muted)",minWidth:72,justifyContent:"center",display:"flex",alignItems:"center",gap:3}}>
                      <IC.Check/><span>Installed</span>
                    </button>
                  ) : prog ? (
                    <div style={{flexShrink:0,minWidth:92,textAlign:"right"}}>
                      <div style={{fontSize:9,color:"var(--accent)",marginBottom:2}}>{prog.message}</div>
                      <div style={{height:3,background:"var(--bg2)",borderRadius:2}}>
                        <div style={{height:"100%",width:`${prog.percent}%`,background:"var(--accent)",borderRadius:2,transition:"width .3s"}}/>
                      </div>
                    </div>
                  ) : (
                    <button onClick={()=>handleInstall(ext)}
                      style={{flexShrink:0,padding:"4px 10px",borderRadius:6,fontSize:10,fontFamily:"inherit",cursor:"pointer",
                        border:"1px solid var(--accent)",background:"rgba(124,106,255,.15)",color:"var(--accent)",minWidth:62,justifyContent:"center",display:"flex",alignItems:"center",gap:3,transition:"all .12s"}}
                      onMouseEnter={e=>e.currentTarget.style.background="rgba(124,106,255,.3)"}
                      onMouseLeave={e=>e.currentTarget.style.background="rgba(124,106,255,.15)"}>
                      + Add
                    </button>
                  )}
                </div>

                {ext.rating > 0 && <Stars r={ext.rating} n={ext.rating_count}/>}

                <div style={{fontSize:10,color:"var(--muted)",marginTop:3,lineHeight:1.4,
                  overflow:"hidden",display:"-webkit-box",WebkitLineClamp:2,WebkitBoxOrient:"vertical"} as React.CSSProperties}>
                  {ext.description}
                </div>

                <div style={{display:"flex",gap:6,marginTop:5,alignItems:"center"}}>
                  <span style={{fontSize:9,padding:"1px 5px",borderRadius:4,background:"rgba(124,106,255,.12)",color:"var(--accent)"}}>{ext.category}</span>
                  <span style={{fontSize:9,color:"var(--disabled)"}}>⬇ {ext.user_count} · {ext.price}</span>
                  {ext.last_updated&&<span style={{fontSize:9,color:"var(--disabled)"}}>· {ext.last_updated}</span>}
                  <a href={ext.store_url} style={{marginLeft:"auto",color:"var(--muted)",display:"flex",alignItems:"center",gap:2,fontSize:9,textDecoration:"none"}}
                    title="Open in Chrome Web Store">
                    <IC.Extern/>
                  </a>
                </div>
              </div>
            </div>
          );
        })}

        {/* Load more */}
        {hasMore && (
          <button onClick={()=>doSearch(q,page+1)}
            style={{width:"100%",padding:"10px",margin:"8px 0",background:"var(--bg2)",border:"1px solid var(--border)",borderRadius:8,color:"var(--muted)",fontSize:12,fontFamily:"inherit",cursor:"pointer"}}>
            {loading?"Loading…":"Load more"}
          </button>
        )}
      </div>

      {/* Footer */}
      <div style={{padding:"6px 14px",borderTop:"1px solid var(--border0)",fontSize:9,color:"var(--muted)",textAlign:"center",flexShrink:0}}>
        Chrome Web Store · MV2+MV3 · Real CRX download + install
      </div>
    </div>
  );
}

// ─── OMNIBOX ─────────────────────────────────────────────────────

function Omnibox({tab,onNavigate,bookmarks,history,onBookmark,certInfo}:{
  tab:Tab;onNavigate:(u:string)=>void;bookmarks:BookmarkItem[];history:HistoryItem[];
  onBookmark:()=>void;certInfo:CertInfo|null;
}) {
  const [focused, setFocused] = useState(false);
  const [value, setValue]     = useState("");
  const [sugs, setSugs]       = useState<Suggestion[]>([]);
  const [sel, setSel]         = useState(-1);
  const [showCert, setShowCert] = useState(false);
  const ref = useRef<HTMLInputElement>(null);
  const isSecure   = tab.url.startsWith("https://");
  const isBookmark = bookmarks.some(b => b.url === tab.url);

  useEffect(() => { if (!focused) setValue(tab.url === "parsec://newtab" ? "" : tab.url); }, [tab.url, focused]);

  const computeSugs = useCallback((q: string) => {
    if (!q.trim()) { setSugs([]); return; }
    const lo = q.toLowerCase();
    const out: Suggestion[] = [];
    bookmarks.filter(b=>b.url.includes(lo)||b.title.toLowerCase().includes(lo)).slice(0,3).forEach(b=>out.push({type:"bookmark",url:b.url,title:b.title,favicon:b.favicon}));
    history.filter(h=>h.url.includes(lo)||h.title.toLowerCase().includes(lo)).slice(0,4).forEach(h=>out.push({type:"history",url:h.url,title:h.title,favicon:h.favicon}));
    out.push({type:"search",url:`https://search.parsec.os/search?q=${encodeURIComponent(q)}`,title:`Search "${q}"`});
    setSugs(out.slice(0,8)); setSel(-1);
    // Async real suggestions
    ipc<{query:string;url:string}[]>("GetSuggestions",{query:q,engine:"Parsec Search"}).then(r=>{
      if (r?.length) setSugs(prev=>[...prev.filter(s=>s.type!=="search"),...r.slice(0,4).map(x=>({type:"search" as const,url:x.url,title:x.query}))]);
    });
  }, [bookmarks, history]);

  const commit = (val: string) => { const url=normalizeUrl(val.trim()); setFocused(false); setValue(url); setSugs([]); onNavigate(url); ref.current?.blur(); };
  const onKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key==="Enter") { commit(sel>=0?sugs[sel].url:value); return; }
    if (e.key==="ArrowDown") { e.preventDefault(); setSel(i=>Math.min(i+1,sugs.length-1)); return; }
    if (e.key==="ArrowUp")   { e.preventDefault(); setSel(i=>Math.max(i-1,-1)); return; }
    if (e.key==="Escape")    { setFocused(false); setSugs([]); ref.current?.blur(); }
  };

  return (
    <div style={{position:"relative",flex:1,maxWidth:740}}>
      <div style={{display:"flex",alignItems:"center",gap:7,background:focused?"var(--bg0)":"var(--bg2)",border:`1.5px solid ${focused?"var(--accent)":"var(--border)"}`,borderRadius:focused&&sugs.length>0?"10px 10px 0 0":10,padding:"5px 10px",transition:"all .12s",boxShadow:focused?"0 0 0 3px rgba(124,106,255,.14)":"none"}}>
        {tab.url!=="parsec://newtab"&&<button onClick={()=>setShowCert(v=>!v)} style={{background:"none",border:"none",cursor:"pointer",padding:0,display:"flex",alignItems:"center",flexShrink:0}}><IC.Lock ok={isSecure}/></button>}
        <input ref={ref}
          value={focused?value:(tab.url==="parsec://newtab"?"":truncUrl(tab.url,90))}
          onChange={e=>{setValue(e.target.value);computeSugs(e.target.value);}}
          onFocus={()=>{setFocused(true);setValue(tab.url==="parsec://newtab"?"":tab.url);ref.current?.select();}}
          onBlur={()=>setTimeout(()=>{setFocused(false);setSugs([]);},160)}
          onKeyDown={onKey}
          placeholder="Search or enter URL…"
          style={{flex:1,background:"transparent",border:"none",outline:"none",fontSize:12.5,color:"var(--fg)",fontFamily:"var(--mono)"}}/>
        {certInfo&&!focused&&tab.url.startsWith("https")&&(
          <span style={{fontSize:9,padding:"1px 5px",borderRadius:4,background:"rgba(74,222,128,.1)",color:"var(--ok)",flexShrink:0,fontFamily:"var(--mono)"}}
            title={`${certInfo.protocol} · ${certInfo.cipher}`}>{certInfo.protocol}</span>
        )}
        <button onClick={onBookmark} style={{background:"none",border:"none",cursor:"pointer",padding:2,color:isBookmark?"var(--accent)":"var(--muted)",display:"flex",alignItems:"center",flexShrink:0,transition:"color .15s"}}><IC.Star f={isBookmark}/></button>
      </div>

      {showCert&&certInfo&&!focused&&(
        <div style={{position:"absolute",top:"calc(100% + 4px)",left:0,zIndex:2000,width:320,background:"var(--bg1)",border:"1px solid var(--border)",borderRadius:10,padding:14,boxShadow:"0 8px 32px rgba(0,0,0,.5)",animation:"fadeIn .12s ease"}}>
          <div style={{display:"flex",alignItems:"center",gap:8,marginBottom:10}}>
            <span style={{fontSize:20}}>{certInfo.is_trusted?"🔒":"⚠️"}</span>
            <div>
              <div style={{fontSize:13,fontWeight:600,color:certInfo.is_trusted?"var(--ok)":"var(--danger)"}}>{certInfo.is_trusted?(certInfo.is_ev?"Extended Validation":"Secure · "+certInfo.protocol):"Not Trusted"}</div>
              <div style={{fontSize:10,color:"var(--muted)"}}>{certInfo.cipher}</div>
            </div>
          </div>
          {[["Subject",certInfo.subject],["Issuer",certInfo.issuer],["Valid until",certInfo.valid_until],["Fingerprint",certInfo.fingerprint]].map(([k,v])=>(
            <div key={k} style={{display:"flex",justifyContent:"space-between",fontSize:11,padding:"3px 0",borderBottom:"1px solid var(--border0)"}}>
              <span style={{color:"var(--muted)"}}>{k}</span>
              <span style={{color:"var(--fg)",fontFamily:"var(--mono)",fontSize:10}}>{v}</span>
            </div>
          ))}
        </div>
      )}

      {focused&&sugs.length>0&&(
        <div style={{position:"absolute",top:"100%",left:0,right:0,zIndex:1000,background:"var(--bg1)",border:"1.5px solid var(--accent)",borderTop:"none",borderRadius:"0 0 10px 10px",overflow:"hidden",boxShadow:"0 8px 24px rgba(0,0,0,.4)"}}>
          {sugs.map((s,i)=>(
            <div key={i} onMouseDown={()=>commit(s.url)} onMouseEnter={()=>setSel(i)}
              style={{display:"flex",alignItems:"center",gap:9,padding:"8px 12px",cursor:"pointer",fontSize:12.5,background:i===sel?"var(--bghov)":"transparent",borderBottom:i<sugs.length-1?"1px solid var(--border0)":"none"}}>
              <span style={{fontSize:12,flexShrink:0}}>{s.type==="search"?"🔍":s.type==="bookmark"?"🔖":s.favicon||"🌐"}</span>
              <div style={{flex:1,minWidth:0}}>
                <div style={{color:"var(--fg)",overflow:"hidden",textOverflow:"ellipsis",whiteSpace:"nowrap"}}>{s.title}</div>
                {s.type!=="search"&&<div style={{color:"var(--muted)",fontSize:10,overflow:"hidden",textOverflow:"ellipsis",whiteSpace:"nowrap"}}>{s.url}</div>}
              </div>
              <span style={{fontSize:9,padding:"2px 5px",borderRadius:4,background:"var(--bg2)",color:"var(--muted)"}}>{s.type==="search"?"Search":s.type==="bookmark"?"Bookmark":"History"}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// ─── TAB BAR ─────────────────────────────────────────────────────

function TabBar({tabs,activeId,onSelect,onClose,onNew,onResume}:{
  tabs:Tab[];activeId:string;onSelect:(id:string)=>void;onClose:(id:string)=>void;onNew:()=>void;onResume:(id:string)=>void;
}) {
  return (
    <div style={{display:"flex",alignItems:"flex-end",gap:2,padding:"5px 8px 0",background:"var(--bg0)",overflowX:"auto",scrollbarWidth:"none",minHeight:36,WebkitAppRegion:"drag"} as React.CSSProperties}>
      {tabs.map(t=>{
        const a=t.id===activeId;
        return (
          <div key={t.id} onClick={()=>t.suspended?onResume(t.id):onSelect(t.id)}
            style={{display:"flex",alignItems:"center",gap:6,padding:"5px 9px 5px 10px",borderRadius:"7px 7px 0 0",
              background:a?"var(--bg1)":"transparent",cursor:"pointer",minWidth:0,maxWidth:196,flex:"0 1 196px",
              border:`1px solid ${a?"var(--border)":"transparent"}`,borderBottom:a?"1px solid var(--bg1)":"1px solid transparent",
              transition:"background .1s",WebkitAppRegion:"no-drag",opacity:t.suspended?0.6:1} as React.CSSProperties}
            onMouseEnter={e=>{if(!a)e.currentTarget.style.background="var(--bghov)";}}
            onMouseLeave={e=>{if(!a)e.currentTarget.style.background="transparent";}}>
            {t.loading?<div style={{width:13,height:13,flexShrink:0}}><IC.Spin/></div>
              :t.suspended?<span style={{flexShrink:0,color:"var(--muted)"}}><IC.Sleep/></span>
              :<span style={{fontSize:11,flexShrink:0}}>{t.incognito?"🕵️":t.favicon||"🌐"}</span>}
            <span style={{fontSize:11,color:a?"var(--fg)":"var(--muted)",overflow:"hidden",textOverflow:"ellipsis",whiteSpace:"nowrap",flex:1}}>
              {t.blocked?"🛡️ Blocked":t.suspended?"💤 "+t.title:t.title||"New Tab"}
            </span>
            {t.audible&&<span style={{fontSize:9}}>{t.muted?"🔇":"🔊"}</span>}
            {!t.pinned&&(
              <button onClick={e=>{e.stopPropagation();onClose(t.id);}}
                style={{background:"none",border:"none",cursor:"pointer",color:"var(--muted)",display:"flex",alignItems:"center",padding:2,borderRadius:3,flexShrink:0,opacity:a?1:0.5}}
                onMouseEnter={e=>{e.currentTarget.style.background="var(--bghov)";e.currentTarget.style.color="var(--fg)";}}
                onMouseLeave={e=>{e.currentTarget.style.background="none";e.currentTarget.style.color="var(--muted)";}}>
                <IC.X/>
              </button>
            )}
          </div>
        );
      })}
      <button onClick={onNew}
        style={{background:"none",border:"none",cursor:"pointer",padding:"5px 9px",color:"var(--muted)",display:"flex",alignItems:"center",borderRadius:"7px 7px 0 0",flexShrink:0,WebkitAppRegion:"no-drag"} as React.CSSProperties}
        onMouseEnter={e=>{e.currentTarget.style.background="var(--bghov)";e.currentTarget.style.color="var(--fg)";}}
        onMouseLeave={e=>{e.currentTarget.style.background="none";e.currentTarget.style.color="var(--muted)";}}>
        <IC.Plus/>
      </button>
    </div>
  );
}

// ─── TOOLBAR ─────────────────────────────────────────────────────

function Toolbar({tab,onBack,onFwd,onReload,onHome,onNavigate,bookmarks,history,onBookmark,onMenu,extensions,certInfo}:{
  tab:Tab;onBack:()=>void;onFwd:()=>void;onReload:()=>void;onHome:()=>void;
  onNavigate:(u:string)=>void;bookmarks:BookmarkItem[];history:HistoryItem[];
  onBookmark:()=>void;onMenu:()=>void;extensions:InstalledExtension[];certInfo:CertInfo|null;
}) {
  const btns=[
    {icon:<IC.Back/>,  action:onBack,   dis:!tab.canGoBack, title:"Back"},
    {icon:<IC.Fwd/>,   action:onFwd,    dis:!tab.canGoFwd,  title:"Forward"},
    {icon:tab.loading?<IC.Spin/>:<IC.Reload/>, action:onReload, dis:false, title:"Reload"},
    {icon:<IC.Home/>,  action:onHome,   dis:false,          title:"Home"},
  ];
  return (
    <div style={{display:"flex",alignItems:"center",gap:6,padding:"5px 10px",background:"var(--bg1)",borderBottom:"1px solid var(--border)",minHeight:40}}>
      {btns.map((b,i)=>(
        <button key={i} onClick={b.action} disabled={b.dis} title={b.title}
          style={{background:"none",border:"none",cursor:b.dis?"not-allowed":"pointer",padding:5,borderRadius:6,color:b.dis?"var(--disabled)":"var(--fg2)",display:"flex",alignItems:"center",transition:"all .1s",flexShrink:0}}
          onMouseEnter={e=>{if(!b.dis){e.currentTarget.style.background="var(--bghov)";e.currentTarget.style.color="var(--fg)";}}}
          onMouseLeave={e=>{e.currentTarget.style.background="none";e.currentTarget.style.color=b.dis?"var(--disabled)":"var(--fg2)";}}>
          {b.icon}
        </button>
      ))}
      <Omnibox tab={tab} onNavigate={onNavigate} bookmarks={bookmarks} history={history} onBookmark={onBookmark} certInfo={certInfo}/>
      <div style={{display:"flex",gap:1,flexShrink:0}}>
        {extensions.filter(e=>e.enabled).slice(0,5).map(e=>(
          <button key={e.id} title={e.name}
            style={{background:"none",border:"none",cursor:"pointer",padding:5,borderRadius:6,fontSize:e.icon.startsWith("data:")?0:13,transition:"all .1s",width:28,height:28,display:"flex",alignItems:"center",justifyContent:"center"}}
            onMouseEnter={ev=>ev.currentTarget.style.background="var(--bghov)"}
            onMouseLeave={ev=>ev.currentTarget.style.background="none"}>
            {e.icon.startsWith("data:")
              ?<img src={e.icon} style={{width:18,height:18,borderRadius:4,objectFit:"cover"}}/>
              :e.icon}
          </button>
        ))}
      </div>
      <button onClick={onMenu} style={{background:"none",border:"none",cursor:"pointer",padding:5,borderRadius:6,color:"var(--fg2)",display:"flex",alignItems:"center",transition:"all .1s",flexShrink:0}}
        onMouseEnter={e=>{e.currentTarget.style.background="var(--bghov)";e.currentTarget.style.color="var(--fg)";}}
        onMouseLeave={e=>{e.currentTarget.style.background="none";e.currentTarget.style.color="var(--fg2)";}}>
        <IC.Menu/>
      </button>
    </div>
  );
}

// ─── NEW TAB PAGE ────────────────────────────────────────────────

function NewTabPage({onNavigate,stats}:{onNavigate:(u:string)=>void;stats:PrivacyStats}) {
  const [q,setQ]=useState(""); const ref=useRef<HTMLInputElement>(null);
  useEffect(()=>{ref.current?.focus();}, []);
  const shortcuts=[
    {name:"GitHub",    url:"https://github.com",            emoji:"🐙"},
    {name:"Claude",    url:"https://claude.ai",             emoji:"🤖"},
    {name:"HN",        url:"https://news.ycombinator.com",  emoji:"🟧"},
    {name:"MDN",       url:"https://developer.mozilla.org", emoji:"📚"},
    {name:"Linear",    url:"https://linear.app",            emoji:"📋"},
    {name:"Figma",     url:"https://figma.com",             emoji:"🎨"},
    {name:"Gmail",     url:"https://mail.google.com",       emoji:"📧"},
    {name:"Calendar",  url:"https://calendar.google.com",   emoji:"📅"},
  ];
  const total_blocked = stats.ads_blocked+stats.trackers_blocked+stats.popups_blocked+stats.miners_blocked;
  return (
    <div style={{width:"100%",height:"100%",background:"var(--bg1)",display:"flex",flexDirection:"column",alignItems:"center",justifyContent:"center",gap:28,color:"var(--fg)"}}>
      <div style={{display:"flex",flexDirection:"column",alignItems:"center",gap:8}}>
        <div style={{width:68,height:68,borderRadius:18,background:"linear-gradient(135deg,#7c6aff,#4a9eff)",display:"flex",alignItems:"center",justifyContent:"center",fontSize:34,boxShadow:"0 8px 32px rgba(124,106,255,.4)"}}>🌐</div>
        <div style={{fontSize:26,fontWeight:700,letterSpacing:-0.5}}>Parsec Web</div>
        <div style={{fontSize:11,color:"var(--muted)"}}>{new Date().toLocaleDateString("en-US",{weekday:"long",month:"long",day:"numeric"})}</div>
      </div>
      <form onSubmit={e=>{e.preventDefault();if(q.trim())onNavigate(normalizeUrl(q));}} style={{width:"100%",maxWidth:560,padding:"0 24px"}}>
        <div style={{display:"flex",alignItems:"center",gap:10,background:"var(--bg2)",border:"1.5px solid var(--border)",borderRadius:14,padding:"11px 16px"}}>
          <span style={{color:"var(--muted)",display:"flex"}}><IC.Srch/></span>
          <input ref={ref} value={q} onChange={e=>setQ(e.target.value)} placeholder="Search or enter URL…"
            style={{flex:1,background:"transparent",border:"none",outline:"none",fontSize:15,color:"var(--fg)",fontFamily:"inherit"}}/>
        </div>
      </form>
      <div style={{display:"grid",gridTemplateColumns:"repeat(8,1fr)",gap:10,maxWidth:600,padding:"0 24px"}}>
        {shortcuts.map(s=>(
          <button key={s.url} onClick={()=>onNavigate(s.url)}
            style={{display:"flex",flexDirection:"column",alignItems:"center",gap:5,padding:"10px 6px",borderRadius:10,border:"none",background:"var(--bg2)",cursor:"pointer",color:"var(--fg)",fontSize:10,fontFamily:"inherit",transition:"background .12s"}}
            onMouseEnter={e=>e.currentTarget.style.background="var(--bghov)"}
            onMouseLeave={e=>e.currentTarget.style.background="var(--bg2)"}>
            <span style={{fontSize:20}}>{s.emoji}</span>
            <span style={{color:"var(--muted)"}}>{s.name}</span>
          </button>
        ))}
      </div>
      <div style={{display:"flex",gap:20,fontSize:11,color:"var(--muted)",borderTop:"1px solid var(--border0)",paddingTop:14,flexWrap:"wrap",justifyContent:"center"}}>
        <span>🛡️ {total_blocked.toLocaleString()} blocked · {fmtB(stats.bytes_saved)} saved</span>
        <span>⚡ HTTP/2 · wgpu Neutron GPU</span>
        <span>🧩 Chrome Web Store · Real CRX install</span>
      </div>
    </div>
  );
}

// ─── SYNC PANEL ──────────────────────────────────────────────────

function SyncPanel({ onClose }: { onClose: () => void }) {
  const [cfg, setCfg]       = useState<{ enabled: boolean; server_url: string; user_id: string } | null>(null);
  const [passphrase, setPassphrase] = useState("");
  const [email, setEmail]   = useState("");
  const [status, setStatus] = useState<string | null>(null);
  const [busy, setBusy]     = useState(false);
  const [tab, setTab]       = useState<"cloud" | "file">("cloud");

  useEffect(() => {
    ipc<{ enabled: boolean; server_url: string; user_id: string }>("SyncGetConfig").then(c => {
      if (c) setCfg(c);
    });
  }, []);

  const save = async () => {
    if (!cfg) return;
    await ipc("SyncSetConfig", { server_url: cfg.server_url, user_id: cfg.user_id, enabled: cfg.enabled });
    setStatus("✅ Config saved");
  };

  const register = async () => {
    if (!email || !passphrase) { setStatus("⚠️ Enter email + passphrase first"); return; }
    setBusy(true); setStatus("Registering…");
    const r = await ipc<{ user_id?: string; error?: string }>("SyncRegister", { email, passphrase });
    setBusy(false);
    setStatus(r?.user_id ? `✅ Registered! ID: ${r.user_id}` : `❌ ${(r as any)?.error || "Failed"}`);
  };

  const push = async () => {
    if (!passphrase) { setStatus("⚠️ Enter your passphrase"); return; }
    setBusy(true); setStatus("Pushing to server…");
    const r = await ipc<{ pushed?: string[]; errors?: string[] }>("SyncPush", { passphrase });
    setBusy(false);
    if (r?.errors?.length) setStatus(`⚠️ Partial: ${r.errors.join(", ")}`);
    else setStatus(`✅ Pushed: ${r?.pushed?.join(", ") || "nothing"}`);
  };

  const pull = async () => {
    if (!passphrase) { setStatus("⚠️ Enter your passphrase"); return; }
    setBusy(true); setStatus("Pulling from server…");
    const r = await ipc<{ bookmarks?: number; history?: number; errors?: string[] }>("SyncPull", { passphrase });
    setBusy(false);
    if (r?.errors?.length) setStatus(`⚠️ Partial: ${r.errors.join(", ")}`);
    else setStatus(`✅ Pulled: ${r?.bookmarks || 0} bookmarks, ${r?.history || 0} history`);
  };

  const exportFile = async () => {
    if (!passphrase) { setStatus("⚠️ Enter passphrase first"); return; }
    const path = `${dirs_download()}/parsec-sync-${Date.now()}.enc`;
    setBusy(true); setStatus("Exporting…");
    const r = await ipc<{ path?: string }>("SyncExportFile", { path, passphrase });
    setBusy(false);
    setStatus(r?.path ? `✅ Saved to ${r.path}` : "❌ Export failed");
  };

  const importFile = async () => {
    if (!passphrase) { setStatus("⚠️ Enter passphrase first"); return; }
    const path = prompt("Path to .enc file:");
    if (!path) return;
    setBusy(true); setStatus("Importing…");
    const r = await ipc<{ bookmarks?: number; history?: number }>("SyncImportFile", { path, passphrase });
    setBusy(false);
    setStatus(r ? `✅ Imported ${r.bookmarks || 0} bookmarks, ${r.history || 0} history` : "❌ Import failed");
  };

  function dirs_download() { return "~/Downloads"; }

  if (!cfg) return <div style={{ padding: 24, color: "var(--muted)" }}>Loading…</div>;

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", overflow: "hidden" }}>
      {/* Header */}
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", padding: "12px 14px", borderBottom: "1px solid var(--border)", flexShrink: 0 }}>
        <span style={{ fontWeight: 600, fontSize: 13, color: "var(--fg)", display: "flex", alignItems: "center", gap: 6 }}>
          <span>🔄</span> Sync
        </span>
        <button onClick={onClose} style={{ background: "none", border: "none", cursor: "pointer", color: "var(--muted)", display: "flex", padding: 4, borderRadius: 5 }}><IC.X /></button>
      </div>

      <div style={{ flex: 1, overflowY: "auto", padding: "0 14px" }}>
        {/* Enable toggle */}
        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", padding: "12px 0", borderBottom: "1px solid var(--border0)" }}>
          <div>
            <div style={{ fontSize: 12, fontWeight: 600, color: "var(--fg)" }}>Sync enabled</div>
            <div style={{ fontSize: 10, color: "var(--muted)", marginTop: 2 }}>E2E encrypted · XChaCha20-Poly1305 + Argon2id</div>
          </div>
          <div onClick={() => setCfg(c => c ? { ...c, enabled: !c.enabled } : c)}
            style={{ width: 34, height: 18, borderRadius: 9, cursor: "pointer", position: "relative", background: cfg.enabled ? "var(--accent)" : "var(--bg2)", border: "1px solid var(--border)", transition: "background .2s" }}>
            <div style={{ position: "absolute", top: 1, width: 14, height: 14, borderRadius: "50%", background: "white", transition: "left .2s", boxShadow: "0 1px 3px rgba(0,0,0,.3)", left: cfg.enabled ? 18 : 1 }} />
          </div>
        </div>

        {/* Tabs */}
        <div style={{ display: "flex", borderBottom: "1px solid var(--border0)", margin: "8px 0 0" }}>
          {([["cloud", "☁️ Cloud sync"], ["file", "📁 File / USB"]] as const).map(([t, label]) => (
            <button key={t} onClick={() => setTab(t)}
              style={{ padding: "8px 12px", background: "none", border: "none", cursor: "pointer", fontSize: 11, fontFamily: "inherit", fontWeight: tab === t ? 600 : 400, color: tab === t ? "var(--accent)" : "var(--muted)", borderBottom: `2px solid ${tab === t ? "var(--accent)" : "transparent"}`, marginBottom: -1, transition: "all .1s" }}>
              {label}
            </button>
          ))}
        </div>

        {tab === "cloud" && (
          <div style={{ paddingTop: 10 }}>
            {/* Server config */}
            <div style={{ fontSize: 10, fontWeight: 600, color: "var(--accent)", textTransform: "uppercase", letterSpacing: 0.8, marginBottom: 6 }}>Server</div>
            <div style={{ marginBottom: 8 }}>
              <div style={{ fontSize: 10, color: "var(--muted)", marginBottom: 3 }}>Sync server URL</div>
              <input value={cfg.server_url}
                onChange={e => setCfg(c => c ? { ...c, server_url: e.target.value } : c)}
                style={{ width: "100%", background: "var(--bg2)", border: "1px solid var(--border)", borderRadius: 6, padding: "6px 9px", color: "var(--fg)", fontSize: 11, fontFamily: "inherit", outline: "none" }} />
              <div style={{ fontSize: 9, color: "var(--disabled)", marginTop: 3 }}>
                Default: https://sync.parsec.os · <a href="#" style={{ color: "var(--accent)" }}>Self-host docs</a>
              </div>
            </div>
            <div style={{ marginBottom: 12 }}>
              <div style={{ fontSize: 10, color: "var(--muted)", marginBottom: 3 }}>Device ID</div>
              <input value={cfg.user_id} readOnly
                style={{ width: "100%", background: "var(--bg2)", border: "1px solid var(--border)", borderRadius: 6, padding: "6px 9px", color: "var(--muted)", fontSize: 10, fontFamily: "var(--mono)", outline: "none", cursor: "text" }} />
            </div>
            <button onClick={save}
              style={{ width: "100%", padding: "6px", marginBottom: 14, background: "rgba(124,106,255,.1)", border: "1px solid var(--accent)", borderRadius: 7, color: "var(--accent)", fontSize: 11, fontFamily: "inherit", cursor: "pointer" }}>
              Save config
            </button>

            {/* Register */}
            <div style={{ fontSize: 10, fontWeight: 600, color: "var(--accent)", textTransform: "uppercase", letterSpacing: 0.8, marginBottom: 6 }}>Account</div>
            <div style={{ marginBottom: 6 }}>
              <div style={{ fontSize: 10, color: "var(--muted)", marginBottom: 3 }}>Email (for new account)</div>
              <input value={email} onChange={e => setEmail(e.target.value)} placeholder="you@example.com" type="email"
                style={{ width: "100%", background: "var(--bg2)", border: "1px solid var(--border)", borderRadius: 6, padding: "6px 9px", color: "var(--fg)", fontSize: 11, fontFamily: "inherit", outline: "none" }} />
            </div>
            <div style={{ marginBottom: 10 }}>
              <div style={{ fontSize: 10, color: "var(--muted)", marginBottom: 3 }}>Encryption passphrase</div>
              <input value={passphrase} onChange={e => setPassphrase(e.target.value)} type="password"
                placeholder="Strong passphrase — never sent to server"
                style={{ width: "100%", background: "var(--bg2)", border: "1px solid var(--border)", borderRadius: 6, padding: "6px 9px", color: "var(--fg)", fontSize: 11, fontFamily: "inherit", outline: "none" }} />
              <div style={{ fontSize: 9, color: "var(--disabled)", marginTop: 3 }}>
                Argon2id key derivation · your key never leaves this device
              </div>
            </div>

            <button onClick={register} disabled={busy}
              style={{ width: "100%", padding: "6px", marginBottom: 6, background: "var(--bg2)", border: "1px solid var(--border)", borderRadius: 7, color: "var(--fg2)", fontSize: 11, fontFamily: "inherit", cursor: busy ? "wait" : "pointer" }}>
              Register new account
            </button>

            {/* Push/Pull */}
            <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 6, marginTop: 6 }}>
              <button onClick={push} disabled={busy}
                style={{ padding: "8px", background: "rgba(74,222,128,.1)", border: "1px solid rgba(74,222,128,.3)", borderRadius: 7, color: "var(--ok)", fontSize: 11, fontFamily: "inherit", cursor: busy ? "wait" : "pointer" }}>
                ↑ Push to cloud
              </button>
              <button onClick={pull} disabled={busy}
                style={{ padding: "8px", background: "rgba(124,106,255,.1)", border: "1px solid var(--accent)", borderRadius: 7, color: "var(--accent)", fontSize: 11, fontFamily: "inherit", cursor: busy ? "wait" : "pointer" }}>
                ↓ Pull from cloud
              </button>
            </div>
          </div>
        )}

        {tab === "file" && (
          <div style={{ paddingTop: 10 }}>
            <div style={{ fontSize: 11, color: "var(--muted)", lineHeight: 1.6, marginBottom: 14 }}>
              Export an encrypted backup file and share it via Dropbox, iCloud, USB, or NFS.
              The file is indecipherable without your passphrase.
            </div>
            <div style={{ marginBottom: 12 }}>
              <div style={{ fontSize: 10, color: "var(--muted)", marginBottom: 3 }}>Encryption passphrase</div>
              <input value={passphrase} onChange={e => setPassphrase(e.target.value)} type="password"
                placeholder="Your passphrase"
                style={{ width: "100%", background: "var(--bg2)", border: "1px solid var(--border)", borderRadius: 6, padding: "6px 9px", color: "var(--fg)", fontSize: 11, fontFamily: "inherit", outline: "none" }} />
            </div>
            <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 6 }}>
              <button onClick={exportFile} disabled={busy}
                style={{ padding: "10px", background: "rgba(74,222,128,.1)", border: "1px solid rgba(74,222,128,.3)", borderRadius: 7, color: "var(--ok)", fontSize: 11, fontFamily: "inherit", cursor: busy ? "wait" : "pointer" }}>
                📦 Export .enc file
              </button>
              <button onClick={importFile} disabled={busy}
                style={{ padding: "10px", background: "rgba(124,106,255,.1)", border: "1px solid var(--accent)", borderRadius: 7, color: "var(--accent)", fontSize: 11, fontFamily: "inherit", cursor: busy ? "wait" : "pointer" }}>
                📂 Import .enc file
              </button>
            </div>
            <div style={{ fontSize: 9, color: "var(--disabled)", textAlign: "center", marginTop: 10 }}>
              XChaCha20-Poly1305 encrypted · safe to store anywhere
            </div>
          </div>
        )}

        {/* Status */}
        {status && (
          <div style={{ margin: "12px 0", padding: "8px 10px", background: "var(--bg2)", borderRadius: 7, fontSize: 11, color: status.startsWith("✅") ? "var(--ok)" : status.startsWith("❌") ? "var(--danger)" : "var(--fg)", border: `1px solid ${status.startsWith("✅") ? "rgba(74,222,128,.3)" : status.startsWith("❌") ? "rgba(248,113,113,.3)" : "var(--border0)"}` }}>
            {status}
          </div>
        )}

        {/* What syncs */}
        <div style={{ marginTop: 16, padding: "10px", background: "var(--bg0)", borderRadius: 8, fontSize: 10, color: "var(--muted)", lineHeight: 1.7 }}>
          <div style={{ fontWeight: 600, color: "var(--fg2)", marginBottom: 4 }}>What syncs</div>
          {["✅ Bookmarks", "✅ History (1,000 most recent)", "✅ Settings & preferences", "✅ Saved sessions", "❌ Passwords (local only)", "❌ Cookies (local only)"].map(item => (
            <div key={item}>{item}</div>
          ))}
        </div>
      </div>
    </div>
  );
}

// ─── SIDE PANEL ──────────────────────────────────────────────────

function SidePanel({panel,onClose,history,downloads,bookmarks,extensions,sessions,
  onNavigate,onToggleExt,onDeleteBm,onOpenDl,onCancelDl,onClearHistory,onRestoreSession,
  onCwsUninstall,stats,prefs,onSetPref}:{
  panel:Panel;onClose:()=>void;history:HistoryItem[];downloads:DownloadItem[];
  bookmarks:BookmarkItem[];extensions:InstalledExtension[];sessions:TabSession[];
  onNavigate:(u:string)=>void;onToggleExt:(id:string)=>void;onDeleteBm:(id:string)=>void;
  onOpenDl:(id:string)=>void;onCancelDl:(id:string)=>void;onClearHistory:()=>void;
  onRestoreSession:(id:string)=>void;onCwsUninstall:(id:string)=>void;
  stats:PrivacyStats;prefs:Record<string,unknown>;onSetPref:(k:string,v:unknown)=>void;
}) {
  const [extTab, setExtTab] = useState<"installed"|"store">("installed");
  const [installedIds, setInstalledIds] = useState(new Set(extensions.map(e=>e.id)));
  const [histSearch, setHistSearch] = useState("");

  useEffect(()=>{ setInstalledIds(new Set(extensions.map(e=>e.id))); }, [extensions]);

  if (panel==="none") return null;

  const titles:Record<Panel,string>={none:"",history:"History",downloads:"Downloads",bookmarks:"Bookmarks",extensions:"Extensions",settings:"Settings",sessions:"Sessions",sync:"Sync"};

  const handleInstall = (id: string) => {
    setInstalledIds(prev => new Set([...prev, id]));
  };
  const handleUninstall = (id: string) => {
    setInstalledIds(prev => { const s=new Set(prev); s.delete(id); return s; });
    onCwsUninstall(id);
  };

  const filteredHistory = histSearch
    ? history.filter(h=>h.url.includes(histSearch.toLowerCase())||h.title.toLowerCase().includes(histSearch.toLowerCase()))
    : history;

  return (
    <div style={{width:350,background:"var(--bg1)",borderLeft:"1px solid var(--border)",display:"flex",flexDirection:"column",overflow:"hidden",flexShrink:0,animation:"slideIn .15s ease"}}>
      {/* Header */}
      {panel !== "sync" && (
      <div style={{display:"flex",alignItems:"center",justifyContent:"space-between",padding:"12px 14px",borderBottom:"1px solid var(--border)",flexShrink:0}}>
        <span style={{fontWeight:600,fontSize:13,color:"var(--fg)"}}>{titles[panel]}</span>
        <button onClick={onClose} style={{background:"none",border:"none",cursor:"pointer",color:"var(--muted)",display:"flex",padding:4,borderRadius:5}}><IC.X/></button>
      </div>
      )}

      {/* Sync (renders its own header) */}
      {panel==="sync"&&<SyncPanel onClose={onClose}/>}

      {/* Extensions sub-tabs */}
      {panel==="extensions"&&(
        <div style={{display:"flex",borderBottom:"1px solid var(--border)",padding:"0 14px",flexShrink:0}}>
          {([["installed",`Installed (${extensions.length})`],["store","🧩 Chrome Store"]] as const).map(([t,label])=>(
            <button key={t} onClick={()=>setExtTab(t)}
              style={{padding:"8px 12px",background:"none",border:"none",cursor:"pointer",fontSize:11,fontFamily:"inherit",fontWeight:extTab===t?600:400,color:extTab===t?"var(--accent)":"var(--muted)",borderBottom:`2px solid ${extTab===t?"var(--accent)":"transparent"}`,marginBottom:-1,transition:"all .1s"}}>
              {label}
            </button>
          ))}
        </div>
      )}

      {/* Content */}
      <div style={{flex:1,overflowY:"auto",display:"flex",flexDirection:"column"}}>

        {/* History */}
        {panel==="history"&&(
          <>
            <div style={{padding:"8px 14px 4px",flexShrink:0}}>
              <div style={{display:"flex",gap:7,alignItems:"center",background:"var(--bg2)",border:"1px solid var(--border)",borderRadius:8,padding:"5px 9px"}}>
                <span style={{color:"var(--muted)",display:"flex"}}><IC.Srch/></span>
                <input value={histSearch} onChange={e=>setHistSearch(e.target.value)} placeholder="Search history…"
                  style={{flex:1,background:"transparent",border:"none",outline:"none",fontSize:11,color:"var(--fg)",fontFamily:"inherit"}}/>
                {histSearch&&<button onClick={()=>setHistSearch("")} style={{background:"none",border:"none",cursor:"pointer",color:"var(--muted)",display:"flex",padding:0}}><IC.X/></button>}
              </div>
            </div>
            {filteredHistory.length===0
              ?<div style={{color:"var(--muted)",fontSize:12,textAlign:"center",paddingTop:32}}>No history</div>
              :filteredHistory.slice(0,200).map(item=>(
              <div key={item.id} onClick={()=>onNavigate(item.url)}
                style={{display:"flex",gap:9,padding:"7px 14px",cursor:"pointer",transition:"background .1s",alignItems:"flex-start"}}
                onMouseEnter={e=>e.currentTarget.style.background="var(--bghov)"}
                onMouseLeave={e=>e.currentTarget.style.background="transparent"}>
                <span style={{fontSize:13,flexShrink:0}}>{item.favicon}</span>
                <div style={{flex:1,minWidth:0}}>
                  <div style={{fontSize:12,color:"var(--fg)",overflow:"hidden",textOverflow:"ellipsis",whiteSpace:"nowrap"}}>{item.title}</div>
                  <div style={{fontSize:10,color:"var(--muted)",overflow:"hidden",textOverflow:"ellipsis",whiteSpace:"nowrap"}}>{item.url}</div>
                </div>
                <div style={{display:"flex",flexDirection:"column",alignItems:"flex-end",gap:2}}>
                  <span style={{fontSize:9,color:"var(--muted)"}}>{ago(item.visit_time)}</span>
                  {(item.visit_count||0)>1&&<span style={{fontSize:9,color:"var(--disabled)"}}>{item.visit_count}×</span>}
                </div>
              </div>
            ))}
            {history.length>0&&(
              <button onClick={onClearHistory}
                style={{margin:"8px 14px",padding:"7px",background:"transparent",border:"1px solid var(--border)",borderRadius:7,color:"var(--danger)",fontSize:11,fontFamily:"inherit",cursor:"pointer"}}>
                🗑️ Clear All History
              </button>
            )}
          </>
        )}

        {/* Downloads */}
        {panel==="downloads"&&(
          <div style={{padding:"0 14px"}}>
            {downloads.length===0?<div style={{color:"var(--muted)",fontSize:12,textAlign:"center",paddingTop:40}}>No downloads</div>
              :downloads.map(dl=>(
              <div key={dl.id} style={{padding:"10px 0",borderBottom:"1px solid var(--border0)"}}>
                <div style={{display:"flex",justifyContent:"space-between",marginBottom:4}}>
                  <div style={{fontSize:12,color:"var(--fg)",overflow:"hidden",textOverflow:"ellipsis",whiteSpace:"nowrap",flex:1}}>{dl.filename}</div>
                  {dl.state==="in_progress"&&<button onClick={()=>onCancelDl(dl.id)} style={{background:"none",border:"none",cursor:"pointer",color:"var(--muted)",padding:"0 4px",fontSize:10}}>✕</button>}
                  {dl.state==="complete"&&<button onClick={()=>onOpenDl(dl.id)} style={{background:"none",border:"none",cursor:"pointer",color:"var(--accent)",padding:"0 4px",fontSize:10}}>Open</button>}
                </div>
                {dl.state==="in_progress"&&<div style={{height:3,background:"var(--bg2)",borderRadius:2,marginBottom:3}}><div style={{height:"100%",width:`${dl.progress}%`,background:"var(--accent)",borderRadius:2,transition:"width .3s"}}/></div>}
                <div style={{fontSize:10,color:"var(--muted)"}}>
                  {dl.state==="complete"?`✅ ${fmtB(dl.size)}`:dl.state==="interrupted"?"❌ Interrupted":`${fmtB(dl.downloaded)} / ${fmtB(dl.size)} · ${fmtSpd(dl.speed_bps)}`}
                </div>
              </div>
            ))}
          </div>
        )}

        {/* Bookmarks */}
        {panel==="bookmarks"&&(bookmarks.length===0?<div style={{color:"var(--muted)",fontSize:12,textAlign:"center",paddingTop:40}}>No bookmarks</div>
          :bookmarks.map(bm=>(
          <div key={bm.id} style={{display:"flex",alignItems:"center",gap:9,padding:"7px 14px",transition:"background .1s"}}
            onMouseEnter={e=>e.currentTarget.style.background="var(--bghov)"}
            onMouseLeave={e=>e.currentTarget.style.background="transparent"}>
            <span style={{fontSize:13,flexShrink:0}}>{bm.favicon}</span>
            <div onClick={()=>onNavigate(bm.url)} style={{flex:1,minWidth:0,cursor:"pointer"}}>
              <div style={{fontSize:12,color:"var(--fg)",overflow:"hidden",textOverflow:"ellipsis",whiteSpace:"nowrap"}}>{bm.title}</div>
              <div style={{fontSize:9,color:"var(--muted)"}}>{bm.folder}</div>
            </div>
            <button onClick={()=>onDeleteBm(bm.id)} style={{background:"none",border:"none",cursor:"pointer",color:"var(--muted)",padding:3,borderRadius:4,display:"flex"}}
              onMouseEnter={e=>{e.currentTarget.style.color="var(--danger)";}}
              onMouseLeave={e=>{e.currentTarget.style.color="var(--muted)";}}>
              <IC.X/>
            </button>
          </div>
        )))}

        {/* Sessions */}
        {panel==="sessions"&&(sessions.length===0
          ?<div style={{color:"var(--muted)",fontSize:12,textAlign:"center",paddingTop:40}}>No saved sessions</div>
          :sessions.map(s=>(
          <div key={s.id} style={{padding:"10px 14px",borderBottom:"1px solid var(--border0)"}}>
            <div style={{display:"flex",justifyContent:"space-between",marginBottom:6}}>
              <div style={{fontSize:12,fontWeight:600,color:"var(--fg)"}}>{s.label}</div>
              <div style={{display:"flex",gap:6,alignItems:"center"}}>
                <span style={{fontSize:9,color:"var(--muted)"}}>{ago(s.saved_at)}</span>
                <button onClick={()=>onRestoreSession(s.id)}
                  style={{padding:"3px 8px",background:"rgba(124,106,255,.15)",border:"1px solid var(--accent)",borderRadius:5,color:"var(--accent)",fontSize:10,fontFamily:"inherit",cursor:"pointer"}}>
                  Restore
                </button>
              </div>
            </div>
            <div style={{display:"flex",flexDirection:"column",gap:2}}>
              {s.tabs.slice(0,5).map((t,i)=>(
                <div key={i} style={{fontSize:10,color:"var(--muted)",overflow:"hidden",textOverflow:"ellipsis",whiteSpace:"nowrap"}}>
                  🌐 {t.title||t.url}
                </div>
              ))}
              {s.tabs.length>5&&<div style={{fontSize:10,color:"var(--disabled)"}}>+{s.tabs.length-5} more tabs</div>}
            </div>
          </div>
        )))}

        {/* Extensions — Installed */}
        {panel==="extensions"&&extTab==="installed"&&(
          <div style={{flex:1}}>
            {extensions.length===0&&<div style={{color:"var(--muted)",fontSize:12,textAlign:"center",paddingTop:32}}>No extensions installed</div>}
            {extensions.map(ext=>(
              <div key={ext.id} style={{padding:"10px 14px",borderBottom:"1px solid var(--border0)"}}>
                <div style={{display:"flex",alignItems:"center",gap:9,marginBottom:5}}>
                  {ext.icon.startsWith("data:")
                    ?<img src={ext.icon} style={{width:34,height:34,borderRadius:8,objectFit:"cover",flexShrink:0}}/>
                    :<div style={{width:34,height:34,borderRadius:8,flexShrink:0,background:ext.iconBg||"var(--bg2)",display:"flex",alignItems:"center",justifyContent:"center",fontSize:16}}>{ext.icon}</div>}
                  <div style={{flex:1}}>
                    <div style={{fontSize:12,fontWeight:600,color:"var(--fg)"}}>{ext.name}</div>
                    <div style={{fontSize:9,color:"var(--muted)"}}>v{ext.version} · MV{ext.mv}</div>
                  </div>
                  <div onClick={()=>onToggleExt(ext.id)}
                    style={{width:34,height:18,borderRadius:9,cursor:"pointer",position:"relative",background:ext.enabled?"var(--accent)":"var(--bg2)",border:"1px solid var(--border)",transition:"background .2s",flexShrink:0}}>
                    <div style={{position:"absolute",top:1,width:14,height:14,borderRadius:"50%",background:"white",transition:"left .2s",boxShadow:"0 1px 3px rgba(0,0,0,.3)",left:ext.enabled?18:1}}/>
                  </div>
                </div>
                <div style={{fontSize:10,color:"var(--muted)",lineHeight:1.45,marginBottom:4}}>{ext.description}</div>
                <div style={{display:"flex",gap:4,flexWrap:"wrap"}}>
                  {ext.permissions.slice(0,3).map(p=><span key={p} style={{fontSize:9,padding:"1px 5px",borderRadius:4,background:"var(--bg2)",color:"var(--muted)"}}>{p}</span>)}
                </div>
              </div>
            ))}
            <div style={{padding:14,textAlign:"center"}}>
              <button onClick={()=>setExtTab("store")}
                style={{padding:"7px 18px",borderRadius:7,border:"1px solid var(--accent)",background:"rgba(124,106,255,.1)",color:"var(--accent)",fontSize:11,fontFamily:"inherit",cursor:"pointer"}}>
                🧩 Open Chrome Web Store
              </button>
            </div>
          </div>
        )}

        {/* Extensions — Chrome Web Store */}
        {panel==="extensions"&&extTab==="store"&&(
          <div style={{flex:1,overflow:"hidden",display:"flex",flexDirection:"column"}}>
            <ChromeWebStore installed={installedIds} onInstall={handleInstall} onUninstall={handleUninstall}/>
          </div>
        )}

        {/* Settings */}
        {panel==="settings"&&(
          <div style={{padding:"0 14px",overflowY:"auto"}}>
            {/* Privacy stats dashboard */}
            <div style={{margin:"12px 0 8px",fontSize:10,fontWeight:600,color:"var(--accent)",textTransform:"uppercase",letterSpacing:0.8}}>Session Stats</div>
            <div style={{display:"grid",gridTemplateColumns:"1fr 1fr",gap:8,marginBottom:16}}>
              {[["🛡️ Ads blocked",stats.ads_blocked],["🕵️ Trackers",stats.trackers_blocked],["🚫 Popups",stats.popups_blocked],["⛏️ Miners",stats.miners_blocked],["💾 Saved",fmtB(stats.bytes_saved)],["📡 Requests",stats.requests_total]].map(([k,v])=>(
                <div key={String(k)} style={{background:"var(--bg2)",borderRadius:8,padding:"8px 10px"}}>
                  <div style={{fontSize:9,color:"var(--muted)"}}>{k}</div>
                  <div style={{fontSize:15,fontWeight:700,color:"var(--fg)",marginTop:2}}>{v}</div>
                </div>
              ))}
            </div>
            {[
              {section:"Privacy & Blocking",items:[
                {label:"Block ads",k:"block_ads"},{label:"Block trackers",k:"block_trackers"},
                {label:"Block NSFW content",k:"block_nsfw"},{label:"Block popups",k:"block_popups"},
                {label:"HTTPS-only mode",k:"https_only"},{label:"Do Not Track",k:"do_not_track"},
              ]},
              {section:"Performance",items:[
                {label:"Auto-suspend background tabs",k:"auto_suspend_tabs"},
                {label:"Neutron GPU acceleration",k:"hardware_accel"},
                {label:"Prefetch pages",k:"prefetch"},
                {label:"Clear data on exit",k:"clear_on_exit"},
              ]},
              {section:"Search Engine",items:[{label:"Default engine",k:"engine",type:"select",opts:["Parsec Search","DuckDuckGo","Google","Bing","Brave"]}]},
            ].map(g=>(
              <div key={g.section} style={{marginBottom:14}}>
                <div style={{fontSize:10,fontWeight:600,color:"var(--accent)",textTransform:"uppercase",letterSpacing:0.8,padding:"10px 0 5px"}}>{g.section}</div>
                {g.items.map((item:any)=>(
                  <div key={item.label} style={{display:"flex",alignItems:"center",justifyContent:"space-between",padding:"7px 0",borderBottom:"1px solid var(--border0)"}}>
                    <span style={{fontSize:12,color:"var(--fg)",display:"flex",alignItems:"center",gap:5}}>
                      {item.label}
                      {item.k==="block_nsfw" && prefs._parental_locked && (
                        <span title="Locked by Parental Controls" style={{fontSize:10,background:"rgba(255,170,0,.18)",color:"#ffaa00",borderRadius:4,padding:"1px 5px",fontWeight:700,cursor:"default"}}>
                          🔒 Parental
                        </span>
                      )}
                    </span>
                    {item.type==="select"
                      ?<select value={String(prefs[item.k]||"")} onChange={e=>onSetPref(item.k,e.target.value)}
                          style={{background:"var(--bg2)",border:"1px solid var(--border)",color:"var(--fg)",borderRadius:5,padding:"3px 6px",fontSize:11,fontFamily:"inherit",cursor:"pointer"}}>
                          {item.opts?.map((o:string)=><option key={o} value={o}>{o}</option>)}
                        </select>
                      :<div
                          onClick={()=>{
                            if(item.k==="block_nsfw" && prefs._parental_locked) return;
                            onSetPref(item.k,!prefs[item.k]);
                          }}
                          title={item.k==="block_nsfw" && prefs._parental_locked ? "Locked by Parental Controls" : undefined}
                          style={{width:34,height:18,borderRadius:9,cursor:prefs._parental_locked&&item.k==="block_nsfw"?"not-allowed":"pointer",position:"relative",background:prefs[item.k]?"var(--accent)":"var(--bg2)",border:"1px solid var(--border)",transition:"background .2s",opacity:prefs._parental_locked&&item.k==="block_nsfw"?0.6:1}}>
                          <div style={{position:"absolute",top:1,width:14,height:14,borderRadius:"50%",background:"white",transition:"left .2s",boxShadow:"0 1px 3px rgba(0,0,0,.3)",left:prefs[item.k]?18:1}}/>
                        </div>
                    }
                  </div>
                ))}
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

// ─── MENU ────────────────────────────────────────────────────────

function AppMenu({open,onClose,onPanel,onNewTab,onNewIncognito,onZoom,zoom}:{
  open:boolean;onClose:()=>void;onPanel:(p:Panel)=>void;onNewTab:()=>void;onNewIncognito:()=>void;onZoom:(d:number)=>void;zoom:number;
}) {
  if (!open) return null;
  const items:any[]=[
    {label:"New Tab",icon:"🗂️",shortcut:"Ctrl+T",action:onNewTab},
    {label:"New Incognito",icon:"🕵️",shortcut:"Ctrl+Shift+N",action:onNewIncognito},
    {divider:true},
    {label:"History",icon:"🕐",shortcut:"Ctrl+H",action:()=>{onPanel("history");onClose();}},
    {label:"Downloads",icon:"⬇️",shortcut:"Ctrl+J",action:()=>{onPanel("downloads");onClose();}},
    {label:"Bookmarks",icon:"🔖",shortcut:"Ctrl+B",action:()=>{onPanel("bookmarks");onClose();}},
    {label:"Extensions",icon:"🧩",action:()=>{onPanel("extensions");onClose();}},
    {label:"Saved Sessions",icon:"📋",action:()=>{onPanel("sessions");onClose();}},
    {label:"Sync",icon:"🔄",action:()=>{onPanel("sync");onClose();}},
    {divider:true},
    {label:"Print…",icon:"🖨️",shortcut:"Ctrl+P",action:()=>window.print()},
    {divider:true},
    {label:"Settings",icon:"⚙️",action:()=>{onPanel("settings");onClose();}},
    {label:"About Parsec Web v1.3",icon:"ℹ️",action:()=>{}},
  ];
  return (
    <>
      <div onClick={onClose} style={{position:"fixed",inset:0,zIndex:998}}/>
      <div style={{position:"fixed",top:78,right:10,zIndex:999,width:230,background:"var(--bg2)",border:"1px solid var(--border)",borderRadius:11,overflow:"hidden",boxShadow:"0 12px 40px rgba(0,0,0,.55)",animation:"fadeIn .1s ease"}}>
        <div style={{display:"flex",alignItems:"center",justifyContent:"space-between",padding:"8px 12px",borderBottom:"1px solid var(--border0)"}}>
          <span style={{fontSize:12,color:"var(--fg2)"}}>Zoom</span>
          <div style={{display:"flex",alignItems:"center",gap:7}}>
            <button onClick={()=>onZoom(-10)} style={{background:"var(--bg1)",border:"1px solid var(--border)",borderRadius:5,color:"var(--fg)",cursor:"pointer",width:22,height:22,display:"flex",alignItems:"center",justifyContent:"center",fontSize:13,fontFamily:"inherit"}}>−</button>
            <span style={{fontSize:11,color:"var(--fg)",minWidth:34,textAlign:"center"}}>{zoom}%</span>
            <button onClick={()=>onZoom(10)} style={{background:"var(--bg1)",border:"1px solid var(--border)",borderRadius:5,color:"var(--fg)",cursor:"pointer",width:22,height:22,display:"flex",alignItems:"center",justifyContent:"center",fontSize:13,fontFamily:"inherit"}}>+</button>
          </div>
        </div>
        {items.map((item,i)=>{
          if(item.divider)return <div key={i} style={{height:1,background:"var(--border0)",margin:"2px 0"}}/>;
          return (
            <div key={i} onClick={item.action} style={{display:"flex",alignItems:"center",gap:9,padding:"8px 12px",cursor:"pointer",fontSize:12,transition:"background .1s"}}
              onMouseEnter={e=>e.currentTarget.style.background="var(--bghov)"}
              onMouseLeave={e=>e.currentTarget.style.background="transparent"}>
              <span style={{fontSize:14,width:18,textAlign:"center"}}>{item.icon}</span>
              <span style={{flex:1,color:"var(--fg)"}}>{item.label}</span>
              {item.shortcut&&<span style={{fontSize:10,color:"var(--muted)"}}>{item.shortcut}</span>}
            </div>
          );
        })}
      </div>
    </>
  );
}

// ─── BLOCKED PAGE ────────────────────────────────────────────────

function BlockedPage({url,reason,onBack}:{url:string;reason:string;onBack:()=>void}) {
  return (
    <div style={{width:"100%",height:"100%",background:"var(--bg1)",display:"flex",flexDirection:"column",alignItems:"center",justifyContent:"center",gap:16,color:"var(--fg)"}}>
      <div style={{fontSize:56}}>🛡️</div>
      <div style={{fontSize:20,fontWeight:700}}>Parsec Shield Blocked This</div>
      <div style={{fontSize:13,color:"var(--muted)",maxWidth:440,textAlign:"center",lineHeight:1.6}}>
        <code style={{color:"var(--danger)",fontSize:11}}>{url}</code><br/><br/>
        Reason: <span style={{color:"var(--accent)"}}>{reason}</span><br/>
        <span style={{fontSize:11}}>Blocked before any bytes left your device.</span>
      </div>
      <div style={{display:"flex",gap:8,flexWrap:"wrap",justifyContent:"center",maxWidth:380}}>
        {["WKContentRuleList (macOS)","Navigation handler","fetch/XHR override"].map(t=>(
          <span key={t} style={{fontSize:10,padding:"3px 8px",borderRadius:20,background:"rgba(74,222,128,.1)",color:"var(--ok)",border:"1px solid rgba(74,222,128,.2)"}}>{t}</span>
        ))}
      </div>
      <button onClick={onBack} style={{padding:"8px 22px",borderRadius:8,border:"1px solid var(--border)",background:"var(--bg2)",color:"var(--fg)",cursor:"pointer",fontSize:13,fontFamily:"inherit"}}>← Go Back</button>
    </div>
  );
}

// ─── ROOT APP ────────────────────────────────────────────────────

export default function ParsecWeb() {
  const [tabs, setTabs]           = useState<Tab[]>([]);
  const [activeId, setActiveId]   = useState<string>("");
  const [panel, setPanel]         = useState<Panel>("none");
  const [menuOpen, setMenuOpen]   = useState(false);
  const [history, setHistory]     = useState<HistoryItem[]>(DEF_HISTORY);
  const [bookmarks, setBookmarks] = useState<BookmarkItem[]>(DEF_BOOKMARKS);
  const [downloads, setDownloads] = useState<DownloadItem[]>([]);
  const [extensions, setExtensions] = useState<InstalledExtension[]>(DEF_EXTS);
  const [sessions, setSessions]   = useState<TabSession[]>([]);
  const [certInfo, setCertInfo]   = useState<CertInfo|null>(null);
  const [zoom, setZoom]           = useState(100);
  const [stats, setStats]         = useState<PrivacyStats>({ ads_blocked:0, trackers_blocked:0, popups_blocked:0, nsfw_blocked:0, miners_blocked:0, bytes_saved:0, requests_total:0 });
  const [prefs, setPrefs]         = useState<Record<string,unknown>>({ block_ads:true, block_trackers:true, block_nsfw:false, block_popups:true, https_only:true, do_not_track:true, prefetch:true, auto_suspend_tabs:true, clear_on_exit:false, engine:"Parsec Search" });

  const activeTab = tabs.find(t=>t.id===activeId) || tabs[0];

  // Wire Rust event handlers
  useEffect(()=>{
    _updateTab = (id,fn) => setTabs(ts=>ts.map(t=>t.id===id?fn(t):t));
    _setCertInfo = setCertInfo;
    return ()=>{ _updateTab=null; _setCertInfo=null; };
  },[]);

  // Init
  useEffect(()=>{
    addTab("parsec://newtab");
    // Load from backend
    ipc<Record<string,unknown>>("GetPrefs").then(p=>{ if(p) setPrefs(p); });
    ipc<BookmarkItem[]>("GetBookmarks").then(b=>{ if(b?.length) setBookmarks(b); });
    ipc<HistoryItem[]>("GetHistory",{limit:200}).then(h=>{ if(h?.length) setHistory(h); });
    ipc<InstalledExtension[]>("CwsListInstalled").then(e=>{ if(e?.length) setExtensions(e); });
    ipc<TabSession[]>("GetSessions").then(s=>{ if(s) setSessions(s); });
    // Poll stats
    const t = setInterval(()=>{
      ipc<PrivacyStats>("GetPrivacyStats").then(s=>{ if(s) setStats(s); });
    }, 5000);
    return ()=>clearInterval(t);
  },[]);

  // Keyboard shortcuts
  useEffect(()=>{
    const h=(e:globalThis.KeyboardEvent)=>{
      if(e.ctrlKey||e.metaKey){
        switch(e.key){
          case "t": e.preventDefault(); addTab(); break;
          case "w": e.preventDefault(); closeTab(activeId); break;
          case "h": e.preventDefault(); setPanel(p=>p==="history"?"none":"history"); break;
          case "j": e.preventDefault(); setPanel(p=>p==="downloads"?"none":"downloads"); break;
          case "b": e.preventDefault(); setPanel(p=>p==="bookmarks"?"none":"bookmarks"); break;
          case "r": e.preventDefault(); reload(); break;
          case "=":case "+": e.preventDefault(); setZoom(z=>Math.min(z+10,300)); break;
          case "-": e.preventDefault(); setZoom(z=>Math.max(z-10,30)); break;
          case "0": e.preventDefault(); setZoom(100); break;
        }
      }
    };
    window.addEventListener("keydown",h);
    return ()=>window.removeEventListener("keydown",h);
  },[activeId]);

  // Notify Rust of viewport size when panel changes
  useEffect(()=>{
    const panelW = panel!=="none" ? 350 : 0;
    ipc("SetViewport",{x:0,y:80,w:window.innerWidth-panelW,h:window.innerHeight-80-24});
  },[panel]);

  // ── Tab management ─────────────────────────────────────────────

  const addTab = async (url="parsec://newtab", incognito=false) => {
    const r = await ipc<{tabId:string}>("NewTab",{url,incognito});
    const id = r?.tabId || genId();
    const t: Tab = {
      id, url, title: url==="parsec://newtab"?"New Tab":"Loading…",
      favicon:"🌐", loading:url!=="parsec://newtab",
      canGoBack:false, canGoFwd:false, pinned:false, muted:false, audible:false,
      incognito, zoom:100, blocked:false, suspended:false,
    };
    setTabs(ts=>[...ts,t]);
    setActiveId(id);
  };

  const closeTab = async (id:string) => {
    if (tabs.length===1) addTab();
    const idx=tabs.findIndex(t=>t.id===id);
    const next=tabs[idx+1]||tabs[idx-1];
    setTabs(ts=>ts.filter(t=>t.id!==id));
    if (activeId===id&&next) { setActiveId(next.id); await ipc("SwitchTab",{tabId:next.id}); }
    ipc("CloseTab",{tabId:id});
  };

  const switchTab = async (id:string) => {
    setActiveId(id);
    await ipc("SwitchTab",{tabId:id});
    setCertInfo(null);
    const t=tabs.find(x=>x.id===id);
    if (t?.url.startsWith("https://")) ipc<CertInfo>("GetCertInfo",{url:t.url}).then(c=>{ if(c) setCertInfo(c); });
  };

  const resumeTab = (id:string) => {
    setTabs(ts=>ts.map(t=>t.id===id?{...t,suspended:false,loading:true}:t));
    ipc("ResumeTab",{tabId:id});
  };

  const navigate = async (tabId:string, rawUrl:string) => {
    const url = rawUrl==="parsec://newtab"?rawUrl:normalizeUrl(rawUrl);
    // Optimistic UI
    setTabs(ts=>ts.map(t=>t.id===tabId?{...t,url,loading:url!=="parsec://newtab",blocked:false,title:url==="parsec://newtab"?"New Tab":"Loading…"}:t));

    const r = await ipc<{url?:string;blocked?:boolean;reason?:string;category?:string}>("Navigate",{tabId,url});
    if (!r) return;

    if (r.blocked) {
      setTabs(ts=>ts.map(t=>t.id===tabId?{...t,blocked:true,loading:false,blockReason:r.reason,title:`Blocked`}:t));
      setStats(s=>({...s,
        ads_blocked:     s.ads_blocked    +(r.reason==="ad"?1:0),
        trackers_blocked:s.trackers_blocked+(r.reason==="tracker"?1:0),
        popups_blocked:  s.popups_blocked +(r.reason==="popup"?1:0),
        nsfw_blocked:    s.nsfw_blocked   +(r.reason==="nsfw"?1:0),
        miners_blocked:  s.miners_blocked +(r.reason==="miner"?1:0),
      }));
      return;
    }

    const finalUrl = r.url || url;
    setTabs(ts=>ts.map(t=>t.id===tabId?{...t,url:finalUrl,loading:true,blocked:false}:t));

    // Update real history from backend
    ipc<HistoryItem[]>("GetHistory",{limit:200}).then(h=>{ if(h?.length) setHistory(h); });

    if (finalUrl.startsWith("https://")) {
      ipc<CertInfo>("GetCertInfo",{url:finalUrl}).then(c=>{ if(c) setCertInfo(c); });
      if (prefs.prefetch) ipc("Prefetch",{url:finalUrl});
    } else { setCertInfo(null); }
  };

  const reload = () => { ipc("Reload",{tabId:activeId}); setTabs(ts=>ts.map(t=>t.id===activeId?{...t,loading:true}:t)); };

  const toggleBookmark = async () => {
    const url=activeTab?.url;
    if (!url||url==="parsec://newtab") return;
    if (bookmarks.some(b=>b.url===url)) {
      const bm = bookmarks.find(b=>b.url===url);
      if (bm) { await ipc("RemoveBookmark",{id:bm.id}); setBookmarks(bs=>bs.filter(b=>b.url!==url)); }
    } else {
      const bm = await ipc<BookmarkItem>("AddBookmark",{url,title:activeTab.title,favicon:activeTab.favicon,folder:"Bookmarks"});
      if (bm) setBookmarks(bs=>[...bs,bm]);
      else setBookmarks(bs=>[...bs,{id:genId(),url,title:activeTab.title,favicon:activeTab.favicon,folder:"Bookmarks"}]);
    }
  };

  const toggleExt = (id:string) => {
    setExtensions(exts=>exts.map(e=>e.id===id?{...e,enabled:!e.enabled}:e));
    ipc("CwsSetEnabled",{ext_id:id,enabled:!extensions.find(e=>e.id===id)?.enabled});
  };

  const uninstallExt = (id:string) => {
    setExtensions(exts=>exts.filter(e=>e.id!==id));
    ipc("CwsUninstall",{ext_id:id});
  };

  const setPref = (k:string,v:unknown) => { setPrefs(p=>({...p,[k]:v})); ipc("SetPref",{key:k,value:v}); };
  const openDl    = (id:string) => ipc("OpenDownload",{id});
  const cancelDl  = (id:string) => { ipc("CancelDownload",{id}); setDownloads(ds=>ds.map(d=>d.id===id?{...d,state:"interrupted"}:d)); };
  const clearHist = () => { ipc("ClearHistory"); setHistory([]); };
  const restoreSess=(id:string)=>{ ipc<{tabs:{url:string;title:string}[]}>("RestoreSession",{session_id:id}).then(r=>{ r?.tabs?.forEach(t=>addTab(t.url)); }); };

  if (!activeTab) return null;

  return (
    <div style={{width:"100vw",height:"100vh",display:"flex",flexDirection:"column",fontFamily:"var(--ui)",background:"var(--bg0)",overflow:"hidden",userSelect:"none"}}>
      <style>{`
        :root{--bg0:#0d0d10;--bg1:#141418;--bg2:#1c1c22;--bghov:#23232c;--fg:#f0f0f4;--fg2:#a8a8b8;--muted:#6b6b7e;--disabled:#35354a;--border:#2a2a35;--border0:#1e1e26;--accent:#7c6aff;--ok:#4ade80;--danger:#f87171;--ui:-apple-system,BlinkMacSystemFont,'Segoe UI',system-ui,sans-serif;--mono:'JetBrains Mono','Cascadia Code','Fira Code',ui-monospace,monospace;}
        *{box-sizing:border-box;margin:0;padding:0;}
        button,input,select{font-family:inherit;}
        select option{background:var(--bg2);color:var(--fg);}
        ::-webkit-scrollbar{width:5px;height:5px;}::-webkit-scrollbar-track{background:transparent;}::-webkit-scrollbar-thumb{background:var(--border);border-radius:3px;}::-webkit-scrollbar-thumb:hover{background:var(--muted);}
        @keyframes spin{to{transform:rotate(360deg);}}
        @keyframes fadeIn{from{opacity:0;transform:translateY(-5px);}to{opacity:1;transform:none;}}
        @keyframes slideIn{from{opacity:0;transform:translateX(10px);}to{opacity:1;transform:none;}}
        @keyframes loadbar{0%{width:0;margin-left:0}50%{width:70%;margin-left:15%}100%{width:0;margin-left:100%}}
      `}</style>

      {/* Drag region */}
      <div style={{height:5,background:"var(--bg0)",WebkitAppRegion:"drag"} as React.CSSProperties}/>

      <TabBar tabs={tabs} activeId={activeId} onSelect={switchTab} onClose={closeTab} onNew={()=>addTab()} onResume={resumeTab}/>

      <Toolbar tab={activeTab}
        onBack={()=>ipc("Back",{tabId:activeId})}
        onFwd={()=>ipc("Forward",{tabId:activeId})}
        onReload={reload}
        onHome={()=>navigate(activeId,"parsec://newtab")}
        onNavigate={url=>navigate(activeId,url)}
        bookmarks={bookmarks} history={history} onBookmark={toggleBookmark}
        onMenu={()=>setMenuOpen(v=>!v)}
        extensions={extensions} certInfo={certInfo}
      />

      {/* Content */}
      <div style={{flex:1,display:"flex",overflow:"hidden",position:"relative"}}>
        <div style={{flex:1,overflow:"hidden",display:"flex",flexDirection:"column",position:"relative"}}>
          {activeTab.url==="parsec://newtab"?(
            <NewTabPage onNavigate={url=>navigate(activeId,url)} stats={stats}/>
          ):activeTab.blocked?(
            <BlockedPage url={activeTab.url} reason={activeTab.blockReason||"Unknown"} onBack={()=>navigate(activeId,"parsec://newtab")}/>
          ):(
            // Transparent — native wry WebView shows through
            <div style={{flex:1,background:"transparent",position:"relative"}}>
              {activeTab.loading&&(
                <div style={{position:"absolute",top:0,left:0,right:0,height:2,zIndex:100,background:"var(--bg2)"}}>
                  <div style={{height:"100%",background:"var(--accent)",borderRadius:1,animation:"loadbar 1s ease-in-out infinite"}}/>
                </div>
              )}
              {activeTab.suspended&&(
                <div style={{position:"absolute",inset:0,display:"flex",flexDirection:"column",alignItems:"center",justifyContent:"center",background:"var(--bg1)",gap:12}}>
                  <div style={{fontSize:40}}>💤</div>
                  <div style={{fontSize:14,color:"var(--muted)"}}>Tab suspended to save RAM</div>
                  <button onClick={()=>resumeTab(activeId)} style={{padding:"8px 20px",borderRadius:8,border:"1px solid var(--accent)",background:"rgba(124,106,255,.15)",color:"var(--accent)",fontSize:13,fontFamily:"inherit",cursor:"pointer"}}>
                    Resume
                  </button>
                </div>
              )}
            </div>
          )}
        </div>

        <SidePanel panel={panel} onClose={()=>setPanel("none")}
          history={history} downloads={downloads} bookmarks={bookmarks} extensions={extensions} sessions={sessions}
          onNavigate={url=>navigate(activeId,url)}
          onToggleExt={toggleExt} onDeleteBm={id=>{ ipc("RemoveBookmark",{id}); setBookmarks(bs=>bs.filter(b=>b.id!==id)); }}
          onOpenDl={openDl} onCancelDl={cancelDl} onClearHistory={clearHist}
          onRestoreSession={restoreSess} onCwsUninstall={uninstallExt}
          stats={stats} prefs={prefs} onSetPref={setPref}
        />
      </div>

      <AppMenu open={menuOpen} onClose={()=>setMenuOpen(false)} onPanel={setPanel}
        onNewTab={()=>{addTab();setMenuOpen(false);}}
        onNewIncognito={()=>{addTab("parsec://newtab",true);setMenuOpen(false);}}
        onZoom={d=>{const nz=Math.max(30,Math.min(300,zoom+d));setZoom(nz);ipc("SetZoom",{tabId:activeId,level:nz/100});}}
        zoom={zoom}
      />

      {/* Status bar */}
      <div style={{display:"flex",alignItems:"center",gap:12,padding:"2px 12px",background:"var(--bg0)",borderTop:"1px solid var(--border0)",fontSize:9.5,color:"var(--muted)"}}>
        <span style={{display:"flex",alignItems:"center",gap:4}}><IC.Shield/>{(stats.ads_blocked+stats.trackers_blocked+stats.miners_blocked).toLocaleString()} blocked</span>
        {/* Show real negotiated protocol from certInfo, not a hardcoded claim */}
        <span>⚡ {certInfo?.protocol ?? "HTTP/2"}</span>
        <span>🌐 wgpu Neutron GPU</span>
        <span>🧩 Chrome Web Store</span>
        {activeTab.url!=="parsec://newtab"&&!activeTab.blocked&&(
          <span style={{color:activeTab.url.startsWith("https")?"var(--ok)":"var(--danger)"}}>
            {activeTab.url.startsWith("https")?"🔒 Secure":"⚠️ Not Secure"}
          </span>
        )}
        {prefs.block_nsfw&&<span>🔞 NSFW Filter</span>}
        <span style={{marginLeft:"auto",fontSize:9}}>Parsec Web 1.3 · Per-tab WebViews · BG Workers · E2E Sync · PMJ JIT</span>
      </div>
    </div>
  );
}
