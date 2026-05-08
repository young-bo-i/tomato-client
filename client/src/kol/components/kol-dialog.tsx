"use client";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/components/ui/dialog";
import { KolMainPanel } from "./kol-main-panel";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export function KolDialog({ open, onOpenChange }: Props) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      {/* width = 90% of viewport (capped at 1600px), height = 90vh —
          scales with the window so wide-table content (e.g. tomato-books)
          has room. The `!` prefixes override shadcn's default
          `sm:max-w-lg`, which would otherwise cap us at 512px. */}
      <DialogContent className="w-[90vw] !max-w-[1600px] sm:!max-w-[1600px] h-[90vh] p-0 flex flex-col">
        <DialogTitle className="sr-only">KOL 工作台</DialogTitle>
        <DialogDescription className="sr-only">
          KOL 账号登录与用户管理
        </DialogDescription>
        <KolMainPanel />
      </DialogContent>
    </Dialog>
  );
}
