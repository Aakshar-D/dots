export function record({ probe, reached, detail }, ctx) {
  ctx.db.exec(
    "INSERT INTO probes (probe, reached, detail, at) VALUES (?, ?, ?, ?) " +
      "ON CONFLICT (probe) DO UPDATE SET reached = excluded.reached, detail = excluded.detail, at = excluded.at",
    [probe, reached ? 1 : 0, detail ?? "", ctx.now],
  );
  return { probe };
}

export function results(input, ctx) {
  return ctx.db
    .query("SELECT probe, reached, detail FROM probes ORDER BY probe", [])
    .map((r) => ({ probe: r.probe, reached: r.reached === 1, detail: r.detail }));
}
