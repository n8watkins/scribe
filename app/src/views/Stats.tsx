import type { BasicStats } from "../backend";
import { StatsCard } from "../components/layout";

export function StatsView({ stats }: { stats: BasicStats }) {
  return (
    <section className="view-grid">
      <article className="panel-card span-2">
        <div className="section-heading compact">
          <h2>Basic Stats</h2>
          <span className="muted">Computed from local history only</span>
        </div>
        <StatsCard expanded stats={stats} />
      </article>
    </section>
  );
}
