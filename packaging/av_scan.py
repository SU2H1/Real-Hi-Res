"""Scan release files with VirusTotal, Kaspersky and Hybrid Analysis, and write
the results (with an explanation of any detection) into the release notes.

    python packaging/av_scan.py --tag v1.8.1 --notes-in body.md --notes-out new.md \\
        --json results.json [--build-url URL] FILE...

Run by .github/workflows/av-scan.yml before every release (on the packages
the release will publish; with --require-results it fails unless VirusTotal
has finished every file, which holds the release back) and once a day for the
latest release, so the results on the release page stay current as vendors
change their signatures.  Only the Python standard library is used.

Keys come from the environment (GitHub repository secrets); a service without
a key is skipped and its column links to the public report instead:

* VT_API_KEY                  VirusTotal API v3 (public API: 4 requests/min,
                              500/day, so requests are spaced 15.5 s apart).
                              Files unknown to VirusTotal are uploaded (up to
                              650 MB via upload_url); known files are
                              re-analysed when their last analysis is older
                              than a day.
* KASPERSKY_OPENTIP_API_KEY   Kaspersky OpenTIP hash lookup / file scan.  The
                              Kaspersky column shows only this, Kaspersky's own
                              verdict; it never borrows Kaspersky's engine row
                              from the VirusTotal report.
* HYBRID_ANALYSIS_API_KEY     Hybrid Analysis API v2: the Windows .exe files
                              are submitted to the sandbox (when it has not
                              seen them) before anything else, and the
                              verdict plus the AV multiscan are read from
                              /overview/<sha256> at the end.

The Hybrid Analysis and OpenTIP response formats are read defensively (several
field names are accepted) and their raw replies are printed to the log, so a
format change shows up there instead of breaking the release page.

A release page describes only the changes of its own version: the "What's new
in X" sections of other versions are dropped from the notes, and with
--whats-new packaging/RELEASE_NOTES.md this version's section is taken from
that file, so a correction made there reaches the published page on the next
refresh.  --notes-only does just that, without scanning anything.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from dataclasses import dataclass, field
from typing import Callable, Dict, List, Optional, Tuple

START = "<!-- av-scan:start -->"
END = "<!-- av-scan:end -->"
USER_AGENT = "Universal Music Player release scanner (+https://github.com/SU2H1/Universal-Music-Player)"
VT_DIRECT_LIMIT = 32 * 1024 * 1024
VT_MAX = 650 * 1024 * 1024
# Engines whose detections are taken as a strong signal in the explanation.
MAJOR_ENGINES = ("microsoft", "kaspersky", "eset", "bitdefender", "sophos", "crowdstrike", "symantec",
                 "avast", "avg", "malwarebytes", "google", "trendmicro", "mcafee", "paloalto", "sentinelone",
                 "fortinet", "f-secure", "gdata", "emsisoft", "drweb")


# ------------------------------------------------------------------ HTTP ---
Transport = Callable[..., Tuple[int, bytes]]


def http(method: str, url: str, headers: Optional[dict] = None, data: Optional[bytes] = None,
         timeout: float = 600) -> Tuple[int, bytes]:
    req = urllib.request.Request(url, data=data, method=method, headers=dict(headers or {}))
    req.add_header("User-Agent", req.get_header("User-agent") or USER_AGENT)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()
    except (urllib.error.URLError, OSError) as e:
        return 0, str(e).encode()


def multipart(fields: Dict[str, str], files: Dict[str, Tuple[str, bytes]]) -> Tuple[bytes, str]:
    boundary = uuid.uuid4().hex
    parts = []
    for k, v in fields.items():
        parts.append(f'--{boundary}\r\nContent-Disposition: form-data; name="{k}"\r\n\r\n{v}\r\n'.encode())
    for k, (name, blob) in files.items():
        parts.append(f'--{boundary}\r\nContent-Disposition: form-data; name="{k}"; filename="{name}"\r\n'
                     "Content-Type: application/octet-stream\r\n\r\n".encode() + blob + b"\r\n")
    parts.append(f"--{boundary}--\r\n".encode())
    return b"".join(parts), f"multipart/form-data; boundary={boundary}"


def _json(body: bytes):
    try:
        return json.loads(body.decode("utf-8", "replace"))
    except ValueError:
        return None


# --------------------------------------------------------------- results ---
@dataclass
class Result:
    status: str                      # clean | detected | suspicious | pending | error | not-configured | n/a
    summary: str                     # short text for the table cell
    link: str = ""
    detections: List[Tuple[str, str]] = field(default_factory=list)  # (engine, label)
    total: int = 0
    note: str = ""


@dataclass
class Target:
    path: str
    name: str
    sha256: str
    size: int
    note: str = ""                   # e.g. "inside RealHiRes-1.8.1-windows-x64.zip"

    @property
    def is_exe(self) -> bool:
        return self.name.lower().endswith(".exe")


def sha256_of(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


# ------------------------------------------------------------- VirusTotal ---
class VirusTotal:
    base = "https://www.virustotal.com/api/v3"

    def __init__(self, key: str, transport: Transport = http, per_minute: float = 4.0,
                 sleep: Callable[[float], None] = time.sleep, clock: Callable[[], float] = time.time):
        self.key, self.transport, self.sleep, self.clock = key, transport, sleep, clock
        self.gap = 60.0 / per_minute + 0.5
        self.last = -1e9

    @staticmethod
    def gui(sha: str) -> str:
        return f"https://www.virustotal.com/gui/file/{sha}"

    def call(self, method: str, url: str, data: Optional[bytes] = None, ctype: Optional[str] = None,
             timeout: float = 600) -> Tuple[int, object]:
        for _ in range(6):
            wait = self.last + self.gap - self.clock()
            if wait > 0:
                self.sleep(wait)
            self.last = self.clock()
            headers = {"x-apikey": self.key, "accept": "application/json"}
            if ctype:
                headers["content-type"] = ctype
            status, body = self.transport(method, url, headers=headers, data=data, timeout=timeout)
            if status == 429:  # quota: wait a minute and try again
                self.sleep(61)
                continue
            return status, _json(body)
        return 429, None

    def report(self, sha: str):
        status, js = self.call("GET", f"{self.base}/files/{sha}")
        return js if status == 200 else (None if status == 404 else {"error": status})

    def submit(self, t: Target, known: Optional[dict]) -> Optional[str]:
        """Start an analysis: upload an unknown file, re-analyse a stale one."""
        if known and "error" not in known:
            last = (known.get("data", {}).get("attributes", {}) or {}).get("last_analysis_date") or 0
            if self.clock() - last < 20 * 3600:
                return None  # analysed in the last 20 hours: the report is current
            status, js = self.call("POST", f"{self.base}/files/{t.sha256}/analyse")
        else:
            if t.size > VT_MAX:
                return None
            url = f"{self.base}/files"
            if t.size > VT_DIRECT_LIMIT:
                status, js = self.call("GET", f"{self.base}/files/upload_url")
                if status != 200 or not isinstance(js, dict) or not js.get("data"):
                    print(f"VirusTotal: no upload URL for {t.name}: {status} {js}")
                    return None
                url = js["data"]
            with open(t.path, "rb") as f:
                body, ctype = multipart({}, {"file": (t.name, f.read())})
            status, js = self.call("POST", url, data=body, ctype=ctype, timeout=1800)
        if status != 200 or not isinstance(js, dict):
            print(f"VirusTotal: could not start an analysis of {t.name}: {status} {js}")
            return None
        return (js.get("data") or {}).get("id")

    def wait(self, analysis_ids: Dict[str, str], deadline: float) -> None:
        pending = dict(analysis_ids)
        while pending and self.clock() < deadline:
            for sha, aid in list(pending.items()):
                status, js = self.call("GET", f"{self.base}/analyses/{aid}")
                st = ((js or {}).get("data", {}).get("attributes", {}) or {}).get("status") if status == 200 else None
                if st == "completed":
                    del pending[sha]
            if pending:
                self.sleep(30)

    @staticmethod
    def parse(t: Target, js) -> Result:
        """The VirusTotal result of a file report.  Kaspersky's engine row in it is
        only part of this result: the Kaspersky column shows Kaspersky's own verdict."""
        link = VirusTotal.gui(t.sha256)
        if not js or "error" in js:
            return Result("pending", "not analysed yet", link)
        attrs = js.get("data", {}).get("attributes", {}) or {}
        stats = attrs.get("last_analysis_stats") or {}
        results = attrs.get("last_analysis_results") or {}
        dets = [(v.get("engine_name") or k, v.get("result") or v.get("category"))
                for k, v in sorted(results.items())
                if v.get("category") in ("malicious", "suspicious")]
        total = sum(int(stats.get(k, 0)) for k in ("malicious", "suspicious", "undetected", "harmless"))
        mal, sus = int(stats.get("malicious", 0)), int(stats.get("suspicious", 0))
        status = "detected" if mal else "suspicious" if sus else "clean"
        summary = f"{mal + sus} / {total}" if total else "no verdicts yet"
        return Result(status if total else "pending", summary, link, dets, total)


# --------------------------------------------------------------- OpenTIP ---
class OpenTIP:
    """Kaspersky Threat Intelligence Portal.  Its API documentation is only
    rendered in a browser, so the reply is read defensively and logged."""

    base = "https://opentip.kaspersky.com/api/v1"

    def __init__(self, key: str, transport: Transport = http):
        self.key, self.transport = key, transport

    @staticmethod
    def gui(sha: str) -> str:
        # The portal's own report links use the upper-case hash.
        return f"https://opentip.kaspersky.com/{sha.upper()}/results?tab=upload"

    def check(self, t: Target, upload: bool) -> Result:
        headers = {"x-api-key": self.key, "accept": "application/json"}
        status, body = self.transport("GET", f"{self.base}/search/hash?request={t.sha256}", headers=headers)
        js = _json(body) if status == 200 else None
        if (js is None or not js) and upload and t.size <= 256 * 1024 * 1024:
            with open(t.path, "rb") as f:
                data = f.read()
            status, body = self.transport(
                "POST", f"{self.base}/scan/file?filename={urllib.parse.quote(t.name)}",
                headers={**headers, "content-type": "application/octet-stream"}, data=data, timeout=1800)
            js = _json(body) if status == 200 else None
        print(f"OpenTIP {t.name}: HTTP {status} {json.dumps(js)[:600] if js is not None else body[:200]!r}")
        if js is None:
            return Result("error" if status not in (0, 404) else "pending",
                          f"no result (HTTP {status})" if status else "not reachable", self.gui(t.sha256))
        zone = str(js.get("Zone") or js.get("zone") or "").strip()
        dets = []
        for d in js.get("DetectionsInfo") or js.get("detectionsInfo") or []:
            if isinstance(d, dict):
                name = d.get("DetectionName") or d.get("detectionName")
                if name:
                    dets.append(("Kaspersky OpenTIP", name))
        if zone.lower() == "red" or dets:
            return Result("detected", f"{zone or 'Red'} zone" + (f": `{dets[0][1]}`" if dets else ""),
                          self.gui(t.sha256), dets, 1)
        if zone.lower() in ("yellow", "orange"):
            return Result("suspicious", f"{zone} zone", self.gui(t.sha256), [("Kaspersky OpenTIP", f"{zone} zone")], 1)
        if zone.lower() == "green":
            return Result("clean", "Green zone", self.gui(t.sha256), [], 1)
        if zone.lower() == "grey":
            return Result("pending", "not categorized by Kaspersky yet (Grey zone)", self.gui(t.sha256))
        return Result("pending", f"{zone or 'no'} verdict", self.gui(t.sha256))


# -------------------------------------------------------- Hybrid Analysis ---
class HybridAnalysis:
    """Falcon Sandbox API v2 (endpoints and form fields as in the official
    VxAPI client).  Replies are read defensively and logged."""

    base = "https://www.hybrid-analysis.com/api/v2"

    def __init__(self, key: str, transport: Transport = http, environment: int = 160,
                 sleep: Callable[[float], None] = time.sleep, clock: Callable[[], float] = time.time):
        self.key, self.transport, self.environment = key, transport, environment
        self.sleep, self.clock = sleep, clock
        self.state: Dict[str, str] = {}   # sha256 -> "known" | "submitted" | "quick" | why not submitted
        self.quick: Dict[str, str] = {}   # sha256 -> quick-scan id
        self._scan_type: Optional[str] = None

    @staticmethod
    def gui(sha: str) -> str:
        return f"https://hybrid-analysis.com/sample/{sha}"

    def call(self, method, path, fields=None, files=None, timeout=600):
        headers = {"api-key": self.key, "accept": "application/json", "User-Agent": "Falcon Sandbox"}
        data = None
        if files is not None:
            data, ctype = multipart(fields or {}, files)
            headers["content-type"] = ctype
        elif fields is not None:
            data = urllib.parse.urlencode(fields).encode()
            headers["content-type"] = "application/x-www-form-urlencoded"
        status, body = self.transport(method, f"{self.base}{path}", headers=headers, data=data, timeout=timeout)
        js = _json(body)
        print(f"Hybrid Analysis {method} {path}: HTTP {status} {json.dumps(js)[:600] if js is not None else body[:200]!r}")
        return status, js

    def start(self, t: Target) -> None:
        """Make sure a sandbox report exists or is being made (submitting the
        file when Hybrid Analysis has never seen it), so it runs while the
        other services are queried."""
        status, _ = self.call("GET", f"/overview/{t.sha256}")
        if status == 200:
            self.state[t.sha256] = "known"
            return
        if status != 404:
            self.state[t.sha256] = f"no report (HTTP {status})"
            return
        with open(t.path, "rb") as f:
            blob = f.read()
        status, js = self.call("POST", "/submit/file", {"environment_id": str(self.environment)},
                               {"file": (t.name, blob)}, timeout=1800)
        if 200 <= status < 300:
            self.state[t.sha256] = "submitted"
            return
        # Free keys may not submit to the sandbox (the official client says file
        # submission needs a premium key); the antivirus quick scan may still work.
        qid = self._quick_scan(t, blob)
        if qid:
            self.state[t.sha256], self.quick[t.sha256] = "quick", qid
        else:
            self.state[t.sha256] = "not in Hybrid Analysis yet (uploading needs a premium API key)"

    def _quick_scan(self, t: Target, blob: bytes) -> Optional[str]:
        if self._scan_type is None:
            status, js = self.call("GET", "/quick-scan/state")
            self._scan_type = self.pick_scan_type(js) or ""
        if not self._scan_type:
            return None
        status, js = self.call("POST", "/quick-scan/file", {"scan_type": self._scan_type}, {"file": (t.name, blob)})
        return str(js.get("id")) if 200 <= status < 300 and isinstance(js, dict) and js.get("id") else None

    @staticmethod
    def pick_scan_type(js) -> Optional[str]:
        """A scan type from /quick-scan/state: "all" when offered, else the first
        available one.  The reply is read defensively (list or {"scanners": [...]})."""
        items = ((js.get("scanners") or js.get("available_scanners") or []) if isinstance(js, dict)
                 else (js or []))
        ids = []
        for it in items if isinstance(items, list) else []:
            if isinstance(it, str):
                ids.append(it)
            elif isinstance(it, dict) and it.get("available", True) is not False:
                ident = it.get("id") or it.get("name")
                if ident:
                    ids.append(str(ident))
        return next((i for i in ids if i.lower() == "all"), ids[0] if ids else None)

    def check(self, t: Target, deadline: float) -> Result:
        """The report's verdict, waiting for a running analysis until `deadline`
        (only when the file is known or was accepted: nothing else will appear)."""
        state = self.state.get(t.sha256, "known")
        while True:
            if state == "quick":
                status, ov = self.call("GET", f"/quick-scan/{self.quick[t.sha256]}")
                done = isinstance(ov, dict) and ov.get("finished")
                res = self.parse(t, ov if status == 200 and done else None, quick=True)
            else:
                status, ov = self.call("GET", f"/overview/{t.sha256}")
                res = self.parse(t, ov if status == 200 else None)
            if res.status != "pending" or self.clock() >= deadline:
                break
            if state not in ("known", "submitted", "quick"):
                break
            self.sleep(30 if state == "quick" else 60)
        if res.status == "pending" and state not in ("known", "submitted", "quick"):
            res.summary = state
        return res

    @staticmethod
    def parse(t: Target, ov, quick: bool = False) -> Result:
        """Result from an /overview reply (the sandbox verdict plus the
        antivirus multiscan in its "scanners" list) or a finished quick scan."""
        link = HybridAnalysis.gui(t.sha256)
        if not isinstance(ov, dict):
            return Result("pending", "sandbox report pending", link)
        verdict = str(ov.get("verdict") or "").strip()
        score = ov.get("threat_score")
        scanners = ov.get("scanners") if isinstance(ov.get("scanners"), list) else []
        v2 = ov.get("scanners_v2")
        if not scanners and isinstance(v2, dict):
            scanners = [dict(v, name=v.get("name") or k) for k, v in v2.items() if isinstance(v, dict)]
        dets: List[Tuple[str, str]] = []
        positives = total = 0
        for sc in scanners:
            if not isinstance(sc, dict):
                continue
            try:
                p, n = int(sc.get("positives") or 0), int(sc.get("total") or 0)
            except (TypeError, ValueError):
                p = n = 0
            state = str(sc.get("status") or "").lower()
            positives, total = positives + p, total + n
            if p or state in ("malicious", "suspicious"):
                dets.append((f"{sc.get('name') or 'scanner'} (Hybrid Analysis)",
                             state if state and not p else f"{p}/{n} engines"))
        parts = [verdict, f"threat score {score}/100" if score is not None else "",
                 (f"AV {positives}/{total}" + (" (quick scan)" if quick else "")) if total else ""]
        text = ", ".join(x for x in parts if x) or "sandbox report pending"
        v = verdict.lower()
        if v == "malicious" or dets:
            return Result("detected" if v == "malicious" or positives else "suspicious", text, link,
                          dets or [("Hybrid Analysis sandbox", verdict)], total)
        if v == "suspicious":
            return Result("suspicious", text, link, [("Hybrid Analysis sandbox", "suspicious")], total)
        if v in ("no specific threat", "whitelisted") or (quick and total):
            return Result("clean", text, link, [], total)
        return Result("pending", text, link)


def classify(detections: List[Tuple[str, str]], total: int) -> str:
    """'isolated' when only a few engines and no major one flag the file."""
    engines = {e for e, _ in detections}
    major = [e for e in engines if any(m in e.lower().replace(" ", "") for m in MAJOR_ENGINES)]
    return "isolated" if len(engines) <= 3 and not major else "serious"


REVIEWS_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "av_reviews.json")


def load_reviews(path: str = REVIEWS_PATH) -> Dict[str, dict]:
    """The maintainer's checks of detections: {sha256: {"date", "engines", "note"}}.
    A missing or broken file means no reviews."""
    try:
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
    except (OSError, ValueError):
        return {}
    return {k.lower(): v for k, v in data.items()
            if isinstance(data, dict) and isinstance(v, dict) and v.get("note") and v.get("engines")}


def explain(t: Target, rows: Dict[str, Result], build_url: str,
            reviews: Optional[Dict[str, dict]] = None) -> Optional[str]:
    dets: List[Tuple[str, str]] = []
    for service, r in rows.items():
        for e, label in r.detections:
            dets.append((e if service == "VirusTotal" else f"{e} ({service})", label))
    if not dets:
        return None
    seen, uniq = set(), []
    for d in dets:
        if d not in seen:
            seen.add(d)
            uniq.append(d)
    names = ", ".join(f"{e} (`{label}`)" for e, label in uniq)
    vt = rows.get("VirusTotal")
    total = vt.total if vt else 0
    built = f"[GitHub Actions]({build_url})" if build_url else "GitHub Actions"
    text = [f"**`{t.name}`**{(' ' + t.note) if t.note else ''} is flagged by {names}"
            + (f", out of {total} VirusTotal engines." if total else ".")]
    review = (reviews or {}).get(t.sha256)
    if review:
        covered = {str(e).lower() for e in review["engines"]}
        new = sorted({e for e, _ in uniq if e.split(" (")[0].lower() not in covered})
        if not new:  # every current detection was looked at by the maintainer
            text.append(f"Checked by the maintainer on {review.get('date', '?')}: {review['note']}")
            return "\n\n".join(text)
        text.append(f"The maintainer checked the detection by {', '.join(review['engines'])} on "
                    f"{review.get('date', '?')}, but {', '.join(new)} flagged the file since, which that "
                    "check does not cover:")
    if classify(uniq, total) == "isolated":
        text.append(
            "Only a few engines flag it, none of them a major vendor, and the names are heuristic or "
            "machine-learning verdicts rather than a known malware family. This is the typical false "
            "positive for programs packaged with PyInstaller, which ship a small launcher and compressed "
            "Python code that such heuristics mistake for a packed program. Universal Music Player contains no "
            f"malware: this file was built by {built} from the public source code of this tag, and its "
            f"SHA-256 is `{t.sha256}`, so you can check that your download is the same file. Such "
            "detections are reported to the vendor as false positives and usually disappear within "
            "days; this table is updated every day.")
    else:
        text.append(
            "Several engines, or a major vendor, flag it. That is more than the usual PyInstaller false "
            "positive and needs a careful manual check by the maintainer. The file was built by "
            f"{built} from the public source code of this tag (SHA-256 `{t.sha256}`). Until this "
            "section says otherwise, prefer to wait for that check, or build the program from source.")
    return "\n\n".join(text)


def cell(r: Optional[Result]) -> str:
    if r is None:
        return "–"
    mark = {"clean": "Clean", "detected": "**Detected**", "suspicious": "**Suspicious**"}.get(r.status)
    text = r.summary if r.status != "clean" or r.summary not in ("clean",) else "Clean"
    if mark and r.status != "clean":
        text = f"{mark}: {r.summary}"
    elif r.status == "clean" and r.summary not in ("clean", "Clean"):
        text = f"Clean ({r.summary})"
    return f"[{text}]({r.link})" if r.link else text


def render(tag: str, targets: List[Target], table: Dict[str, Dict[str, Result]], build_url: str,
           now: Optional[dt.datetime] = None, configured: Optional[Dict[str, bool]] = None,
           reviews: Optional[Dict[str, dict]] = None) -> str:
    now = now or dt.datetime.now(dt.timezone.utc)
    configured = configured or {}
    lines = [START, "## Antivirus scan results", "",
             f"Last checked {now:%Y-%m-%d %H:%M} UTC. The files of this release are scanned with VirusTotal "
             "(70+ engines), Kaspersky and the Hybrid Analysis sandbox, and the results are refreshed every "
             "day, because vendors keep changing their signatures. Click a result for the full report.", "",
             "| File | SHA-256 | VirusTotal | Kaspersky | Hybrid Analysis |",
             "| --- | --- | --- | --- | --- |"]
    for t in targets:
        row = table.get(t.sha256, {})
        name = f"`{t.name}`" + (f" ({t.note})" if t.note else "")
        lines.append(f"| {name} | `{t.sha256[:12]}…` | {cell(row.get('VirusTotal'))} | "
                     f"{cell(row.get('Kaspersky'))} | {cell(row.get('Hybrid Analysis'))} |")
    missing = [s for s, ok in configured.items() if not ok]
    if missing:
        lines += ["", f"Not scanned automatically yet: {', '.join(missing)} (no API key configured); the links "
                  "open the public report for each file's SHA-256, if anyone has submitted it."]
    reviews = load_reviews() if reviews is None else reviews
    notes = [n for n in (explain(t, table.get(t.sha256, {}), build_url, reviews) for t in targets) if n]
    scanned = any(r.status in ("clean", "detected", "suspicious") for row in table.values() for r in row.values())
    lines += ["", "### Detections explained", ""]
    if notes:
        lines.append("\n\n".join(notes))
    elif scanned:
        lines.append("No engine reports any of these files as malicious or suspicious in the results above.")
    else:
        lines.append("There are no automatic scan results yet, so nothing can be said here; use the links "
                     "above to see whether a service has a report for these files.")
    lines += ["", "Full SHA-256 checksums: " + ", ".join(f"`{t.name}` `{t.sha256}`" for t in targets), END]
    return "\n".join(lines)


WHATS_NEW = re.compile(r"^## What's new in (\S+)[ \t]*$", re.M)
HEADING = re.compile(r"^## ", re.M)


def _whats_new_sections(text: str) -> List[Tuple[str, int, int]]:
    """(version, start, end) of every "What's new in X" section; one ends at the
    next level-2 heading or at the antivirus section."""
    out = []
    for m in WHATS_NEW.finditer(text):
        nxt = HEADING.search(text, m.end())
        end = nxt.start() if nxt else len(text)
        mark = text.find(START, m.end())
        if mark != -1:
            end = min(end, mark)
        out.append((m.group(1), m.start(), end))
    return out


def whats_new_section(text: str, version: str) -> Optional[str]:
    """This version's "What's new" section of `text`, or None."""
    for v, a, b in _whats_new_sections(text):
        if v == version:
            return text[a:b].rstrip() + "\n\n"
    return None


def only_this_version(body: str, version: str, current: Optional[str] = None) -> str:
    """Release notes that describe only `version`: the "What's new" sections of
    other versions are dropped, and `current` (this version's section from
    RELEASE_NOTES.md) replaces this version's own, or goes first if it has none."""
    parts, pos, found = [], 0, False
    for v, a, b in _whats_new_sections(body):
        parts.append(body[pos:a])
        if v == version:
            parts.append(current if current is not None else body[a:b])
            found = True
        pos = b
    parts.append(body[pos:])
    out = "".join(parts)
    if current is not None and not found:
        out = current + out.lstrip("\n")
    return out


def merge_notes(body: str, section: str) -> str:
    """Replace the scan section of the release notes, or append it."""
    if START in body and END in body:
        a = body.index(START)
        b = body.index(END, a) + len(END)
        return body[:a] + section + body[b:]
    return body.rstrip() + "\n\n" + section + "\n"


# ------------------------------------------------------------------- main ---
def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("files", nargs="*")
    ap.add_argument("--tag", required=True)
    ap.add_argument("--notes-in", required=True)
    ap.add_argument("--notes-out", required=True)
    ap.add_argument("--json", help="write the raw results here")
    ap.add_argument("--build-url", default="")
    ap.add_argument("--inner", action="append", default=[],
                    help="NAME=ARCHIVE: mark a file as unpacked from a release archive")
    ap.add_argument("--minutes", type=float, default=45.0, help="time budget for waiting on analyses")
    ap.add_argument("--sandbox-minutes", type=float, default=20.0,
                    help="at most this long (from the start) for Hybrid Analysis sandbox reports")
    ap.add_argument("--opentip-upload-all", action="store_true",
                    help="also upload the release archives to Kaspersky OpenTIP when it does not know them "
                         "(slow, so only the daily refresh does it; the .exe files are always uploaded)")
    ap.add_argument("--whats-new", help="RELEASE_NOTES.md: this version's \"What's new\" section is taken from it")
    ap.add_argument("--notes-only", action="store_true",
                    help="only make the notes describe this version alone; scan nothing, keep the antivirus section")
    ap.add_argument("--require-results", action="store_true",
                    help="exit 1 unless VirusTotal has a finished result for every file (used before a release)")
    args = ap.parse_args(argv)
    version = args.tag[1:] if args.tag.startswith("v") else args.tag
    current = None
    if args.whats_new:
        with open(args.whats_new, encoding="utf-8") as f:
            current = whats_new_section(f.read().replace("<version>", version), version)
    if args.whats_new and current is None and args.require_results:
        print(f"::error::{args.whats_new} has no \"What's new in {version}\" section, so the release page "
              "would not describe this version.")
        return 1
    with open(args.notes_in, encoding="utf-8") as f:
        body = only_this_version(f.read(), version, current)
    if args.notes_only:
        with open(args.notes_out, "w", encoding="utf-8") as f:
            f.write(body)
        print(body)
        return 0
    if not args.files:
        ap.error("no files to scan")

    inner = dict(x.split("=", 1) for x in args.inner)
    targets = []
    for p in args.files:
        name = os.path.basename(p)
        targets.append(Target(p, name, sha256_of(p), os.path.getsize(p),
                              f"inside `{inner[name]}`" if name in inner else ""))
    started = time.time()
    deadline = started + args.minutes * 60
    table: Dict[str, Dict[str, Result]] = {t.sha256: {} for t in targets}
    configured = {}

    # The sandbox needs minutes per file, so it gets the files first.
    ha_key = os.environ.get("HYBRID_ANALYSIS_API_KEY", "").strip()
    configured["Hybrid Analysis"] = bool(ha_key)
    ha = HybridAnalysis(ha_key, environment=int(os.environ.get("HYBRID_ANALYSIS_ENVIRONMENT", "160"))) \
        if ha_key else None
    exes = [t for t in targets if t.is_exe]  # the sandbox runs Windows programs; archives go to VirusTotal
    for t in exes if ha else []:
        try:
            ha.start(t)
        except Exception as exc:  # noqa: BLE001 - one service must not break the report
            print(f"Hybrid Analysis {t.name}: {exc!r}")

    vt_key = os.environ.get("VT_API_KEY", "").strip()
    configured["VirusTotal"] = bool(vt_key)
    if vt_key:
        vt = VirusTotal(vt_key, per_minute=float(os.environ.get("VT_REQUESTS_PER_MINUTE", "4")))
        known = {t.sha256: vt.report(t.sha256) for t in targets}
        ids = {}
        for t in targets:
            aid = vt.submit(t, known[t.sha256])
            if aid:
                ids[t.sha256] = aid
        vt.wait(ids, deadline)
        for t in targets:
            js = vt.report(t.sha256) if t.sha256 in ids or not known[t.sha256] else known[t.sha256]
            table[t.sha256]["VirusTotal"] = VirusTotal.parse(t, js)
    else:
        for t in targets:
            table[t.sha256]["VirusTotal"] = Result("not-configured", "open report", VirusTotal.gui(t.sha256))

    kas_key = os.environ.get("KASPERSKY_OPENTIP_API_KEY", "").strip()
    if kas_key:
        tip = OpenTIP(kas_key)
        for t in targets:
            try:
                r = tip.check(t, upload=t.is_exe or args.opentip_upload_all)
            except Exception as exc:  # noqa: BLE001 - one service must not break the report
                print(f"OpenTIP {t.name}: {exc!r}")
                r = Result("error", "no result", OpenTIP.gui(t.sha256))
            table[t.sha256]["Kaspersky"] = r
    configured["Kaspersky"] = bool(kas_key)
    for t in targets:
        table[t.sha256].setdefault("Kaspersky", Result("not-configured", "open report", OpenTIP.gui(t.sha256)))

    for t in exes:
        if ha is None:
            table[t.sha256]["Hybrid Analysis"] = Result("not-configured", "open report", HybridAnalysis.gui(t.sha256))
            continue
        try:
            table[t.sha256]["Hybrid Analysis"] = ha.check(t, min(deadline, started + args.sandbox_minutes * 60))
        except Exception as exc:  # noqa: BLE001
            table[t.sha256]["Hybrid Analysis"] = Result("error", f"error: {exc}", HybridAnalysis.gui(t.sha256))

    section = render(args.tag, targets, table, args.build_url, configured=configured)
    with open(args.notes_out, "w", encoding="utf-8") as f:
        f.write(merge_notes(body, section))
    if args.json:
        with open(args.json, "w", encoding="utf-8") as f:
            json.dump({t.name: {"sha256": t.sha256, **{s: r.__dict__ for s, r in table[t.sha256].items()}}
                       for t in targets}, f, indent=2)
    print(section)
    flagged = [t.name for t in targets if any(r.detections for r in table[t.sha256].values())]
    if flagged:
        print("::warning::Antivirus detections for " + ", ".join(flagged))
    if args.require_results:
        missing = missing_results(targets, table, configured)
        if missing:
            print("::error::No finished antivirus result yet, so the release is held back: " + "; ".join(missing)
                  + ". Re-run the failed job to scan again.")
            return 1
    return 0


def missing_results(targets: List[Target], table: Dict[str, Dict[str, Result]],
                    configured: Dict[str, bool]) -> List[str]:
    """What keeps a release from being published: VirusTotal must be set up
    and must have a finished verdict for every file."""
    if not configured.get("VirusTotal"):
        return ["VT_API_KEY is not set"]
    done = ("clean", "detected", "suspicious")
    return [f"{t.name} (VirusTotal: {table[t.sha256].get('VirusTotal').summary})" for t in targets
            if getattr(table[t.sha256].get("VirusTotal"), "status", "") not in done]


if __name__ == "__main__":
    sys.exit(main())
