import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, onCatalogProgress, onDownloads } from "../shared/api";
import type { Candidate, Catalogue, CatalogProgress, Decision, DiscoverPage, Job, Snapshot } from "../shared/types";
import { Button, Select, bytes } from "./ui";

export interface DiscoverIntent {
  source?: string;
  museum?: string;
  text?: string;
}

const SHORT: Record<string, string> = {
  nga: "National Gallery of Art",
  cma: "Cleveland",
  rijks: "Rijksmuseum",
  met: "The Met",
  aic: "Art Institute of Chicago",
  commons: "Wikimedia Commons",
};

/** Download state of a candidate, if it is (or was) in the queue. */
function jobLabel(j: Job): string {
  switch (j.state) {
    case "queued":
      return "Queued";
    case "checking":
      return "Checking license…";
    case "downloading":
      return j.total ? `Downloading ${Math.round((j.received / j.total) * 100)}%` : `Downloading ${bytes(j.received)}`;
    case "processing":
      return "Saving…";
    case "done":
      return "Saved";
    case "failed":
      return "Failed";
    case "skipped":
      return "Skipped";
  }
}

export function useDownloads() {
  const [jobs, setJobs] = useState<Job[]>([]);
  useEffect(() => {
    let alive = true;
    void api.downloads().then((j) => alive && setJobs(j));
    const un = onDownloads((j) => alive && setJobs(j));
    return () => {
      alive = false;
      void un.then((f) => f());
    };
  }, []);
  return jobs;
}

export function Discover({ snap, intent, goSources }: { snap: Snapshot; intent?: DiscoverIntent; goSources: () => void }) {
  const dl = snap.settings.download;
  const [catalogue, setCatalogue] = useState<Catalogue | null>(null);
  const [source, setSource] = useState(intent?.source ?? "nga");
  const [museum, setMuseum] = useState(intent?.museum ?? "");
  const [text, setText] = useState(intent?.text ?? "");
  const [page, setPage] = useState<DiscoverPage | null>(null);
  const [items, setItems] = useState<Candidate[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [focus, setFocus] = useState<Candidate | null>(null);
  const [progress, setProgress] = useState<CatalogProgress | null>(null);
  const [preparing, setPreparing] = useState(false);
  const jobs = useDownloads();
  const request = useRef(0);

  const refreshCatalogue = useCallback(() => api.catalogue().then(setCatalogue), []);
  useEffect(() => {
    void refreshCatalogue();
    const un = onCatalogProgress(setProgress);
    return () => void un.then((f) => f());
  }, [refreshCatalogue]);

  const run = useCallback(
    async (opts: { source: string; text: string; museum: string; cursor?: string | null }) => {
      const mine = ++request.current;
      setLoading(true);
      setError(null);
      if (!opts.cursor) {
        setItems([]);
        setPage(null);
        setSelected(new Set());
        setFocus(null);
      }
      try {
        const p = await api.discover({
          source: opts.source,
          text: opts.text,
          cursor: opts.cursor ?? null,
          museum: opts.source === "commons" && opts.museum ? opts.museum : null,
          landscapeOnly: dl.landscapeOnly,
          minLongEdge: dl.minLongEdge,
        });
        if (mine !== request.current) return;
        setPage(p);
        setItems((prev) => {
          if (!opts.cursor) return p.items;
          const seen = new Set(prev.map((c) => c.id));
          return [...prev, ...p.items.filter((c) => !seen.has(c.id))];
        });
      } catch (e) {
        if (mine === request.current) setError(String(e));
      } finally {
        if (mine === request.current) setLoading(false);
      }
    },
    [dl.landscapeOnly, dl.minLongEdge],
  );

  // A new source, collection or filter starts a new search with what is typed.
  useEffect(() => {
    void run({ source, text, museum });
  }, [source, museum, run]); // eslint-disable-line react-hooks/exhaustive-deps

  const rule = catalogue?.rules.sources.find((s) => s.id === source);
  const catalog = catalogue?.catalogs.find((c) => c.source === source);
  const jobById = useMemo(() => new Map(jobs.map((j) => [j.id, j])), [jobs]);
  const saved = (c: Candidate) => c.inLibrary || jobById.get(c.id)?.state === "done";
  const busy = (c: Candidate) => {
    const j = jobById.get(c.id);
    return !!j && (j.state === "queued" || j.state === "checking" || j.state === "downloading" || j.state === "processing");
  };
  const selectable = (c: Candidate) => !!c.decision?.allowed && !saved(c) && !busy(c);
  const chosen = items.filter((c) => selected.has(c.id) && selectable(c));
  const collections = snap.museums.filter((m) => !m.direct && m.name !== "Collection not recorded");
  const active = jobs.filter((j) => j.state === "queued" || j.state === "checking" || j.state === "downloading" || j.state === "processing");
  const finished = jobs.length - active.length;

  const toggle = (c: Candidate) => {
    setFocus(c);
    if (!selectable(c)) return;
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(c.id)) next.delete(c.id);
      else next.add(c.id);
      return next;
    });
  };
  const start = async (list: Candidate[]) => {
    if (!list.length) return;
    await api.download(list);
    setSelected((prev) => {
      const next = new Set(prev);
      for (const c of list) next.delete(c.id);
      return next;
    });
  };
  const prepare = async () => {
    setPreparing(true);
    setError(null);
    try {
      await api.prepareCatalog(source);
      await refreshCatalogue();
      await run({ source, text, museum });
    } catch (e) {
      setError(String(e));
    } finally {
      setPreparing(false);
      setProgress(null);
    }
  };

  if (!snap.library.root) {
    return (
      <div className="empty">
        <h2>Choose your art folder first</h2>
        <p className="muted">Downloads are saved next to your other paintings, so Galeria needs to know where they live. Set the folder in Library.</p>
      </div>
    );
  }

  const showAside = !!focus || jobs.length > 0;
  return (
    <div className={`browse discover${showAside ? " with-detail" : ""}`}>
      <div className="browse-main">
        <div className="source-tabs">
          {(catalogue?.rules.sources ?? []).map((s) => (
            <button key={s.id} type="button" data-source={s.id} className={source === s.id ? "active" : ""} onClick={() => setSource(s.id)} title={s.name}>
              {SHORT[s.id] ?? s.name}
            </button>
          ))}
        </div>
        {rule && (
          <div className="source-note">
            <span className="chip ok">
              {rule.grants ? (catalogue?.rules.licenses.find((l) => l.id === rule.grants)?.label ?? rule.grants) : rule.id === "commons" ? "License checked per file" : "Rights checked per record"}
            </span>
            <span title={rule.summary}>
              {rule.summary.split(/(?<=\.)\s/)[0]}{" "}
              <button type="button" className="link" onClick={() => void api.openUrl(rule.policy_url)}>
                {/policy$/i.test(rule.program) ? rule.program : `${rule.program} policy`} ↗
              </button>{" "}
              <button type="button" className="link" onClick={goSources}>
                How Galeria decides
              </button>
            </span>
          </div>
        )}
        <div className="toolbar">
          <input
            className="search"
            data-test="discover-search"
            placeholder={source === "commons" ? "Search Wikimedia Commons: an artist, a title, a place…" : `Search ${rule?.name ?? "the museum"}: an artist, a title, a subject…`}
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void run({ source, text: e.currentTarget.value, museum });
            }}
          />
          <Button kind="primary" onClick={() => void run({ source, text, museum })}>
            <span data-test="discover-go">Search</span>
          </Button>
          {source === "commons" && (
            <Select value={museum} onChange={setMuseum} options={[{ value: "", label: "Any collection" }, ...collections.map((m) => ({ value: m.name, label: m.name }))]} />
          )}
          <span className="inline filters">
          <label className="check compact" title="Hide works that are taller than they are wide">
            <input
              type="checkbox"
              checked={dl.landscapeOnly}
              onChange={(e) => void api.patch({ download: { landscapeOnly: e.target.checked } })}
            />
            Landscape format
          </label>
          <Select
            value={dl.minLongEdge}
            onChange={(v) => void api.patch({ download: { minLongEdge: v } })}
            options={[
              { value: 0, label: "Any size" },
              { value: 2500, label: "2,500 px +" },
              { value: 3000, label: "3,000 px +" },
              { value: 4000, label: "4,000 px +" },
              { value: 5000, label: "5,000 px +" },
            ]}
          />
          </span>
        </div>
        <div className="genre-chips">
          {(catalogue?.genres ?? []).map((g) => (
            <button
              key={g.id}
              type="button"
              className={text.trim().toLowerCase() === g.search.toLowerCase() ? "active" : ""}
              onClick={() => {
                setText(g.search);
                void run({ source, text: g.search, museum });
              }}
            >
              {g.name}
            </button>
          ))}
        </div>

        {page?.needsCatalog ? (
          <div className="card pad catalog-card">
            <h3>Get the National Gallery's catalogue</h3>
            <p className="muted">
              The National Gallery of Art has no search service. It publishes its whole catalogue as open data instead, including which images are open access. Galeria downloads it once ({catalog?.sizeHint ?? "about 35 MB"}), keeps a small index of the open-access paintings and searches that on this computer.
            </p>
            {preparing && progress ? (
              <div className="progress-row">
                <div className="bar">
                  <span style={{ width: `${Math.min(100, (progress.received / Math.max(1, progress.total)) * 100)}%` }} />
                </div>
                <span className="muted small">
                  {progress.stage} {progress.total > 1 ? `· ${bytes(progress.received)}` : ""}
                </span>
              </div>
            ) : (
              <Button kind="primary" onClick={() => void prepare()} disabled={preparing}>
                <span data-test="catalog-get">{preparing ? "Starting…" : "Download Catalogue"}</span>
              </Button>
            )}
          </div>
        ) : (
          <>
            <div className="result-line muted small" data-test="discover-status">
              {loading && !items.length
                ? "Searching…"
                : error
                  ? ""
                  : page
                    ? [
                        `${items.length} shown`,
                        page.total != null ? `${page.total.toLocaleString()} at the source` : null,
                        page.hidden > 0 ? `${page.hidden.toLocaleString()} hidden by the format and size filters` : null,
                        catalog?.ready ? `catalogue of ${catalog.count.toLocaleString()} open-access paintings` : null,
                      ]
                        .filter(Boolean)
                        .join(" · ")
                    : ""}
              {catalog?.ready && (
                <button type="button" className="link" onClick={() => void prepare()} disabled={preparing}>
                  {preparing ? (progress ? `${progress.stage}…` : "Refreshing…") : "Refresh catalogue"}
                </button>
              )}
            </div>
            {error && <div className="error-row">{error}</div>}
            {page?.notice && <div className="notice">{page.notice}</div>}
            <div className="grid" data-test="discover-grid">
              {items.map((c) => {
                const j = jobById.get(c.id);
                const ok = !!c.decision?.allowed;
                return (
                  <button
                    key={c.id}
                    type="button"
                    data-test="cand"
                    data-id={c.id}
                    data-license={c.decision?.license}
                    className={`tile${selected.has(c.id) ? " picked" : ""}${focus?.id === c.id ? " selected" : ""}${ok ? "" : " refused"}${saved(c) ? " owned" : ""}`}
                    onClick={() => toggle(c)}
                  >
                    <div className="tile-img">
                      {c.thumbUrl && <img src={c.thumbUrl} loading="lazy" decoding="async" referrerPolicy="no-referrer" alt="" />}
                      {saved(c) ? <span className="badge">In library</span> : j ? <span className={`badge ${j.state === "failed" ? "bad" : "busy"}`}>{jobLabel(j)}</span> : null}
                      {selectable(c) && <span className={`tick${selected.has(c.id) ? " on" : ""}`}>{selected.has(c.id) ? "✓" : ""}</span>}
                      <span className={`lic ${ok ? "ok" : "no"}`}>{ok ? c.decision!.license : "Not open"}</span>
                      {c.width && c.height ? <span className="dims">{c.width}×{c.height}</span> : null}
                    </div>
                    <div className="tile-title">{c.title}</div>
                    <div className="tile-sub">{[c.artist, c.date].filter(Boolean).join(", ") || c.museum}</div>
                  </button>
                );
              })}
              {!loading && page && !items.length && !error && (
                <div className="muted pad grid-empty">
                  Nothing matched{page.hidden > 0 ? `: ${page.hidden.toLocaleString()} results were hidden by the format and size filters` : ""}. Try another word, or relax the filters.
                </div>
              )}
              {page?.next && (
                <div className="grid-more">
                  <Button onClick={() => void run({ source, text, museum, cursor: page.next })} disabled={loading}>
                    <span data-test="discover-more">{loading ? "Loading…" : "Load More"}</span>
                  </Button>
                </div>
              )}
            </div>
          </>
        )}

        <div className="action-bar">
          <span className="muted">
            {chosen.length ? `${chosen.length} selected` : "Click paintings to select them"}
            {active.length > 0 && ` · downloading ${active.length}`}
          </span>
          <span className="spacer" />
          {items.some(selectable) && (
            <Button kind="ghost" onClick={() => setSelected(new Set(items.filter(selectable).map((c) => c.id)))}>
              Select All
            </Button>
          )}
          {chosen.length > 0 && (
            <Button kind="ghost" onClick={() => setSelected(new Set())}>
              Clear
            </Button>
          )}
          <Button kind="primary" disabled={!chosen.length} onClick={() => void start(chosen)}>
            <span data-test="discover-download">{chosen.length > 1 ? `Download ${chosen.length} Paintings` : "Download"}</span>
          </Button>
        </div>
      </div>

      {showAside && (
        <aside className="detail">
          {jobs.length > 0 && (
            <div className="downloads" data-test="downloads">
              <div className="downloads-head">
                <h3>Downloads</h3>
                {active.length > 0 && (
                  <button type="button" className="link" onClick={() => void api.cancelDownload()}>
                    Cancel all
                  </button>
                )}
                {finished > 0 && (
                  <button type="button" className="link" onClick={() => void api.clearDownloads()}>
                    Clear finished
                  </button>
                )}
              </div>
              {jobs.map((j) => (
                <div key={j.id} className={`job job-${j.state}`} data-test="job">
                  {j.thumbUrl ? <img src={j.thumbUrl} referrerPolicy="no-referrer" alt="" /> : <span className="job-ph" />}
                  <div className="job-text">
                    <div className="job-title">{j.title}</div>
                    <div className="job-state">
                      {j.state === "failed" ? j.error : j.state === "skipped" ? j.reason : j.state === "done" ? `Saved · ${j.width}×${j.height} · ${bytes(j.bytes)} · ${j.license}` : jobLabel(j)}
                    </div>
                    {j.state === "downloading" && (
                      <div className="bar">
                        <span style={{ width: j.total ? `${(j.received / j.total) * 100}%` : "35%" }} className={j.total ? "" : "indeterminate"} />
                      </div>
                    )}
                  </div>
                  {(j.state === "queued" || j.state === "downloading" || j.state === "checking") && (
                    <button type="button" className="job-x" title="Cancel" onClick={() => void api.cancelDownload(j.id)}>
                      ×
                    </button>
                  )}
                </div>
              ))}
            </div>
          )}
          {focus && (
            <div className="detail-body">
              <button type="button" className="close" onClick={() => setFocus(null)}>
                ×
              </button>
              <CandidateDetail
                c={focus}
                job={jobById.get(focus.id)}
                saved={saved(focus)}
                canDownload={selectable(focus)}
                onDownload={() => void start([focus])}
              />
            </div>
          )}
        </aside>
      )}
    </div>
  );
}

export function LicenseBox({ d, notes }: { d: Decision; notes?: string[] }) {
  return (
    <div className={`license-box ${d.allowed ? "ok" : "no"}`} data-test="license-box">
      <div className="license-head">
        <span className={`chip ${d.allowed ? "ok" : "no"}`}>{d.license}</span>
        <strong>{d.allowed ? "Free to download" : "Not downloadable"}</strong>
      </div>
      <p>{d.reason}</p>
      {d.obligations.length > 0 && (
        <ul>
          {d.obligations.map((o) => (
            <li key={o}>{o}</li>
          ))}
        </ul>
      )}
      {notes?.map((n) => (
        <p key={n} className="muted">
          {n}
        </p>
      ))}
      <div className="evidence" title="The fields of the museum's own record this decision rests on">
        {d.evidence}
      </div>
      <div className="license-links">
        {d.licenseUrl && (
          <button type="button" className="link" onClick={() => void api.openUrl(d.licenseUrl!)}>
            License text ↗
          </button>
        )}
        {d.policyUrl && (
          <button type="button" className="link" onClick={() => void api.openUrl(d.policyUrl)}>
            {d.program || "Policy"} ↗
          </button>
        )}
      </div>
    </div>
  );
}

function CandidateDetail({ c, job, saved, canDownload, onDownload }: { c: Candidate; job?: Job; saved: boolean; canDownload: boolean; onDownload: () => void }) {
  const rows: [string, string | null | undefined][] = [
    ["Artist", [c.artist, c.artistBio && `(${c.artistBio})`].filter(Boolean).join(" ")],
    ["Date", c.date],
    ["Medium", c.medium],
    ["Dimensions", c.dimensions],
    ["Collection", [c.museum, c.creditLine].filter(Boolean).join(" · ")],
    ["Accession", c.accession],
    [
      "File",
      [c.width && c.height ? `${c.width}×${c.height} px · ${((c.width * c.height) / 1e6).toFixed(1)} MP` : "Size is known once downloaded", c.downloadBytes ? `${bytes(c.downloadBytes)} download` : null]
        .filter(Boolean)
        .join(" · "),
    ],
  ];
  return (
    <div className="details">
      {(c.previewUrl ?? c.thumbUrl) && <img className="detail-img" src={(c.previewUrl ?? c.thumbUrl)!} referrerPolicy="no-referrer" alt="" />}
      <h3>{c.title}</h3>
      {c.titleOriginal && <div className="muted italic">{c.titleOriginal}</div>}
      <dl>
        {rows
          .filter(([, v]) => v)
          .map(([k, v]) => (
            <div key={k}>
              <dt>{k}</dt>
              <dd>{v}</dd>
            </div>
          ))}
      </dl>
      {c.description && <p className="description">{c.description}</p>}
      {c.decision && <LicenseBox d={c.decision} notes={c.notes} />}
      <div className="actions">
        <Button kind="primary" disabled={!canDownload} onClick={onDownload}>
          {saved ? "In Your Library" : job && job.state !== "failed" && job.state !== "skipped" ? jobLabel(job) : "Download"}
        </Button>
        {c.pageUrl && <Button onClick={() => void api.openUrl(c.pageUrl!)}>Museum Page ↗</Button>}
      </div>
      {job?.state === "failed" && <div className="error-row">{job.error}</div>}
    </div>
  );
}
