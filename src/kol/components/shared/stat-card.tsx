"use client";

import React from "react";

/// Tone controls the card's border/background tint and the value color.
/// All colors are theme-controlled semantic classes (no hardcoded Tailwind
/// palette classes), per the project theming rules.
export type Tone = "neutral" | "success" | "destructive" | "warning" | "muted";

const TONE_CLASS: Record<Tone, string> = {
  neutral: "border-border bg-card",
  success: "border-success/40 bg-success/5",
  destructive: "border-destructive/40 bg-destructive/5",
  warning: "border-warning/40 bg-warning/5",
  muted: "border-border bg-muted/30",
};

const VALUE_CLASS: Record<Tone, string> = {
  neutral: "text-foreground",
  success: "text-success",
  destructive: "text-destructive",
  warning: "text-warning",
  muted: "text-muted-foreground",
};

/// A single labelled counter tile used across the stats dashboards.
/// Memoized because the dashboards re-render on a 30s poll and the tiles'
/// inputs rarely change.
export const StatCard = React.memo(function StatCard({
  label,
  value,
  tone = "neutral",
}: {
  label: string;
  value: number;
  tone?: Tone;
}) {
  return (
    <div className={`rounded-md border px-3 py-2 ${TONE_CLASS[tone]}`}>
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className={`mt-1 font-mono text-2xl tabular-nums ${VALUE_CLASS[tone]}`}>
        {value.toLocaleString()}
      </div>
    </div>
  );
});
