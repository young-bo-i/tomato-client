"use client";

import {
  Dialog,
  DialogContent,
} from "@/components/ui/dialog";
import { KolMainPanel } from "./kol-main-panel";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export function KolDialog({ open, onOpenChange }: Props) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-5xl h-[85vh] p-0 flex flex-col">
        <KolMainPanel />
      </DialogContent>
    </Dialog>
  );
}
