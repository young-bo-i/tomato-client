"use client";

import React from "react";

/// The inline error banner used across the KOL panels. Pass the message as
/// children, e.g. `<ErrorBanner>{error}</ErrorBanner>`. Uses semantic
/// `destructive` theme colors (no hardcoded palette classes).
export function ErrorBanner({ children }: { children: React.ReactNode }) {
  return (
    <div className="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive">
      {children}
    </div>
  );
}
