"use client";

import { cn } from "@/lib/utils";

export interface NavItem {
  value: string;
  label: string;
}

export interface NavGroup {
  label: string;
  items: NavItem[];
}

interface Props {
  groups: NavGroup[];
  active: string;
  onChange: (value: string) => void;
}

export function KolSideNav({ groups, active, onChange }: Props) {
  return (
    <nav className="w-40 shrink-0 border-r flex flex-col gap-1 py-3 px-2 overflow-y-auto">
      {groups.map((group, gi) => (
        <div key={group.label} className={cn(gi > 0 && "mt-3")}>
          <p className="px-2 mb-1 text-[11px] font-semibold text-muted-foreground uppercase tracking-wider">
            {group.label}
          </p>
          {group.items.map((item) => (
            <button
              key={item.value}
              onClick={() => onChange(item.value)}
              className={cn(
                "w-full text-left px-3 py-1.5 rounded-md text-sm transition-colors",
                active === item.value
                  ? "bg-primary text-primary-foreground font-medium"
                  : "text-foreground hover:bg-muted",
              )}
            >
              {item.label}
            </button>
          ))}
        </div>
      ))}
    </nav>
  );
}
