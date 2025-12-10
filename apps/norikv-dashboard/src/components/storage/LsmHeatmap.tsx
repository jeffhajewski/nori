"use client";

import { useMemo } from "react";
import { motion } from "framer-motion";
import type { LsmNodeState, LevelState, SlotState } from "@/stores/eventStore";
import { cn } from "@/lib/utils";

interface LsmHeatmapProps {
  nodeState: LsmNodeState;
  onSlotHover?: (level: number, slot: number, heat: number) => void;
  onSlotClick?: (level: number, slot: number) => void;
}

export function LsmHeatmap({ nodeState, onSlotHover, onSlotClick }: LsmHeatmapProps) {
  const levels = useMemo(() => {
    return Array.from(nodeState.levels.entries())
      .sort(([a], [b]) => a - b)
      .map(([level, state]) => ({
        level,
        slots: Array.from(state.slots.values()).sort((a, b) => a.slotId - b.slotId),
      }));
  }, [nodeState.levels]);

  if (levels.length === 0) {
    return (
      <div className="flex h-64 items-center justify-center rounded-lg border border-dashed border-border">
        <p className="text-sm text-muted-foreground">
          No LSM data yet. Waiting for SlotHeat events...
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-2">
      {levels.map(({ level, slots }) => (
        <LevelRow
          key={level}
          level={level}
          slots={slots}
          onSlotHover={onSlotHover}
          onSlotClick={onSlotClick}
        />
      ))}

      {/* Heat scale legend */}
      <div className="mt-4 flex items-center gap-2">
        <span className="text-xs text-muted-foreground">Cold</span>
        <div className="flex h-3 flex-1 overflow-hidden rounded">
          {Array.from({ length: 20 }).map((_, i) => (
            <div
              key={i}
              className="flex-1"
              style={{ backgroundColor: heatToColor(i / 19) }}
            />
          ))}
        </div>
        <span className="text-xs text-muted-foreground">Hot</span>
      </div>
    </div>
  );
}

interface LevelRowProps {
  level: number;
  slots: SlotState[];
  onSlotHover?: (level: number, slot: number, heat: number) => void;
  onSlotClick?: (level: number, slot: number) => void;
}

function LevelRow({ level, slots, onSlotHover, onSlotClick }: LevelRowProps) {
  const levelColors = [
    "border-red-500/50",
    "border-orange-500/50",
    "border-amber-500/50",
    "border-yellow-500/50",
    "border-green-500/50",
    "border-teal-500/50",
    "border-blue-500/50",
  ];

  return (
    <div className="flex items-center gap-3">
      {/* Level label */}
      <div
        className={cn(
          "flex h-8 w-12 items-center justify-center rounded border-l-4 bg-muted/50 text-sm font-medium",
          levelColors[Math.min(level, 6)]
        )}
      >
        L{level}
      </div>

      {/* Slots grid */}
      <div className="flex flex-1 flex-wrap gap-0.5">
        {slots.map((slot) => (
          <SlotCell
            key={slot.slotId}
            slot={slot}
            level={level}
            onHover={onSlotHover}
            onClick={onSlotClick}
          />
        ))}
      </div>

      {/* Slot count */}
      <div className="w-16 text-right text-xs text-muted-foreground">
        {slots.length} slots
      </div>
    </div>
  );
}

interface SlotCellProps {
  slot: SlotState;
  level: number;
  onHover?: (level: number, slot: number, heat: number) => void;
  onClick?: (level: number, slot: number) => void;
}

function SlotCell({ slot, level, onHover, onClick }: SlotCellProps) {
  const bgColor = heatToColor(slot.heat);
  const isHot = slot.heat > 0.7;

  return (
    <motion.div
      className={cn(
        "h-6 w-6 cursor-pointer rounded-sm transition-all hover:scale-110 hover:ring-2 hover:ring-primary/50",
        isHot && "animate-pulse-subtle"
      )}
      style={{ backgroundColor: bgColor }}
      initial={{ opacity: 0, scale: 0.8 }}
      animate={{ opacity: 1, scale: 1 }}
      transition={{ duration: 0.2 }}
      onMouseEnter={() => onHover?.(level, slot.slotId, slot.heat)}
      onClick={() => onClick?.(level, slot.slotId)}
      title={`Slot ${slot.slotId}: heat=${(slot.heat * 100).toFixed(0)}%, k=${slot.k}`}
    />
  );
}

// Inferno-inspired color scale
function heatToColor(heat: number): string {
  // Clamp heat to [0, 1]
  const h = Math.max(0, Math.min(1, heat));

  if (h < 0.2) {
    // Deep purple to purple
    const t = h / 0.2;
    return interpolateColor([13, 8, 45], [72, 12, 100], t);
  } else if (h < 0.4) {
    // Purple to dark red
    const t = (h - 0.2) / 0.2;
    return interpolateColor([72, 12, 100], [150, 27, 55], t);
  } else if (h < 0.6) {
    // Dark red to orange
    const t = (h - 0.4) / 0.2;
    return interpolateColor([150, 27, 55], [220, 90, 30], t);
  } else if (h < 0.8) {
    // Orange to yellow
    const t = (h - 0.6) / 0.2;
    return interpolateColor([220, 90, 30], [250, 180, 50], t);
  } else {
    // Yellow to white-yellow
    const t = (h - 0.8) / 0.2;
    return interpolateColor([250, 180, 50], [252, 255, 164], t);
  }
}

function interpolateColor(c1: number[], c2: number[], t: number): string {
  const r = Math.round(c1[0] + (c2[0] - c1[0]) * t);
  const g = Math.round(c1[1] + (c2[1] - c1[1]) * t);
  const b = Math.round(c1[2] + (c2[2] - c1[2]) * t);
  return `rgb(${r}, ${g}, ${b})`;
}
