import { useEffect, useState } from "react";
import { api } from "../shared/api";
import type { Catalogue, Snapshot } from "../shared/types";
import type { DiscoverIntent } from "./Discover";
import { Button, Row, Section, Select, Toggle } from "./ui";

/** Museums on/off, and the licensing rules Galeria downloads by. */
export function Sources({ snap, discover }: { snap: Snapshot; discover: (intent: DiscoverIntent) => void }) {
  const [catalogue, setCatalogue] = useState<Catalogue | null>(null);
  useEffect(() => {
    void api.catalogue().then(setCatalogue);
  }, []);
  const disabled = snap.settings.sources.disabledMuseums;
  const setEnabled = (name: string, on: boolean) =>
    void api.patch({ sources: { disabledMuseums: on ? disabled.filter((m) => m !== name) : [...disabled, name] } });
  const off = snap.museums.filter((m) => !m.enabled);
  const shown = snap.museums.filter((m) => m.enabled).reduce((n, m) => n + m.count, 0);
  const rules = catalogue?.rules;
  const label = (id: string | null) => rules?.licenses.find((l) => l.id === id)?.label ?? id ?? "";

  return (
    <>
      <Section
        title="Museums in Your Library"
        description={`Switch a museum off to leave its paintings out of the rotation. Nothing is deleted. ${shown} of ${snap.library.valid} paintings are from museums that are on.`}
        actions={
          off.length > 0 ? (
            <Button onClick={() => void api.patch({ sources: { disabledMuseums: [] } })}>Turn All On</Button>
          ) : undefined
        }
      >
        {snap.museums.map((m) => (
          <div key={m.name} className={`museum-row${m.enabled ? "" : " off"}`} data-test="museum-row">
            <Toggle checked={m.enabled} onChange={(v) => setEnabled(m.name, v)} />
            <div className="museum-text">
              <div className="museum-name">
                {m.name} <span className="muted">· {m.count} {m.count === 1 ? "painting" : "paintings"}</span>
              </div>
              <div className="row-hint">
                {m.direct ? (
                  <>
                    <span className="chip ok">{m.license ?? "Open access"}</span> {m.program} programme: Galeria can download from this museum directly.
                  </>
                ) : (
                  <>
                    <span className="chip">{m.license ?? "No open license"}</span> {m.summary}
                  </>
                )}{" "}
                {m.policyUrl && (
                  <button type="button" className="link" onClick={() => void api.openUrl(m.policyUrl!)}>
                    Its terms ↗
                  </button>
                )}
              </div>
            </div>
            {m.name !== "Collection not recorded" && (
              <Button onClick={() => discover(m.direct && m.source ? { source: m.source } : { source: "commons", museum: m.name })}>
                {m.direct ? "Find More…" : "On Commons…"}
              </Button>
            )}
          </div>
        ))}
        {snap.museums.length === 0 && <div className="muted pad">No museums yet: the manifest names none.</div>}
      </Section>

      <Section title="Downloads" description="How Galeria saves what you pick in Discover.">
        <Row label="Largest size to keep" hint="Bigger originals are scaled down to this long edge. Files are saved as JPEG next to your other paintings and added to manifest.json with their license.">
          <Select
            value={snap.settings.download.maxLongEdge}
            onChange={(v) => void api.patch({ download: { maxLongEdge: v } })}
            options={[
              { value: 4000, label: "4,000 px" },
              { value: 6000, label: "6,000 px" },
              { value: 8000, label: "8,000 px" },
              { value: 12000, label: "12,000 px" },
              { value: 16000, label: "16,000 px" },
            ]}
          />
        </Row>
        <Row label="Find paintings" hint="Browse thumbnails from each museum and download the ones you like.">
          <Button kind="primary" onClick={() => discover({})}>
            Open Discover
          </Button>
        </Row>
      </Section>

      {rules && (
        <>
          <Section title="What Galeria Is Allowed to Download" description={`Rules version ${rules.version}, reviewed ${rules.reviewed}. They are a file compiled into the app (resources/licensing.json).`}>
            <div className="pad prose">
              <p>{rules.policy}</p>
              <div className="license-cols">
                <div>
                  <h4>Accepted</h4>
                  {rules.licenses
                    .filter((l) => l.allowed)
                    .map((l) => (
                      <div key={l.id} className="license-item">
                        <span className="chip ok">{l.label}</span>
                        <span className="muted small">{l.obligations.join(" ") || "No conditions."}</span>
                      </div>
                    ))}
                </div>
                <div>
                  <h4>Refused</h4>
                  {rules.licenses
                    .filter((l) => !l.allowed)
                    .map((l) => (
                      <div key={l.id} className="license-item">
                        <span className="chip no">{l.label}</span>
                        <span className="muted small">{l.reason}</span>
                      </div>
                    ))}
                  <div className="license-item">
                    <span className="chip no">Anything else</span>
                    <span className="muted small">A license Galeria does not recognise, or none stated, is refused.</span>
                  </div>
                </div>
              </div>
              <p className="muted small">{rules.disclaimer}</p>
            </div>
          </Section>

          <Section title="Open-Access Sources" description="The programmes Galeria downloads from, and the field of each museum's own record that proves a work is open.">
            {rules.sources.map((s) => (
              <div key={s.id} className="source-card" data-test="source-card">
                <div className="source-head">
                  <strong>{s.name}</strong>
                  <span className="chip ok">{s.grants ? label(s.grants) : "Per file"}</span>
                  <span className="muted small">{s.program}</span>
                  <span className="spacer" />
                  <Button onClick={() => discover({ source: s.id })}>Browse…</Button>
                </div>
                <p>{s.summary}</p>
                {s.quote && <blockquote>“{s.quote}”</blockquote>}
                <dl className="source-facts">
                  <div>
                    <dt>Proof checked</dt>
                    <dd>
                      <code>{s.evidence}</code>
                    </dd>
                  </div>
                  {s.attribution.requested && (
                    <div>
                      <dt>Credit</dt>
                      <dd>
                        {s.attribution.required ? "Required: " : "Requested, not required: "}
                        {s.attribution.requested}
                      </dd>
                    </div>
                  )}
                  <div>
                    <dt>Images</dt>
                    <dd>{s.images}</dd>
                  </div>
                  {s.etiquette.note && (
                    <div>
                      <dt>Manners</dt>
                      <dd>{s.etiquette.note}</dd>
                    </div>
                  )}
                </dl>
                <div className="license-links">
                  <button type="button" className="link" onClick={() => void api.openUrl(s.policy_url)}>
                    Policy ↗
                  </button>
                  {s.data_url && (
                    <button type="button" className="link" onClick={() => void api.openUrl(s.data_url)}>
                      Data & API ↗
                    </button>
                  )}
                </div>
              </div>
            ))}
          </Section>

          <Section title="Museums Without an Open-Access Programme" description="Galeria never downloads from these. Their works are offered only as Wikimedia Commons files whose own license passes the rules above.">
            {rules.museums.map((m) => (
              <div key={m.name} className="museum-row plain">
                <div className="museum-text">
                  <div className="museum-name">
                    {m.name} {m.license && <span className="chip">{m.license}</span>}
                  </div>
                  <div className="row-hint">
                    {m.summary}{" "}
                    {m.policy_url && (
                      <button type="button" className="link" onClick={() => void api.openUrl(m.policy_url!)}>
                        Its terms ↗
                      </button>
                    )}
                  </div>
                </div>
              </div>
            ))}
            <div className="museum-row plain">
              <div className="museum-text">
                <div className="museum-name">Any other institution</div>
                <div className="row-hint">{rules.fallback.summary}</div>
              </div>
            </div>
          </Section>
        </>
      )}
    </>
  );
}
