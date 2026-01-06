"use client";

import { useMemo, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import type { LsmNodeState, SlotState } from "@/stores/eventStore";
import { useSlotHeatHistory } from "@/stores/eventStore";
import { cn } from "@/lib/utils";
import { Sparkline } from "@/components/charts";

interface LsmHeatmapProps {
  nodeState: LsmNodeState;
  onSlotHover?: (level: number, slot: number, heat: number) => void;
  onSlotClick?: (level: number, slot: number) => void;
}

interface SelectedSlot {
  level: number;
  slotId: number;
  heat: number;
  k: number;
}

export function LsmHeatmap({ nodeState, onSlotHover, onSlotClick }: LsmHeatmapProps) {
  const [selectedSlot, setSelectedSlot] = useState<SelectedSlot | null>(null);

  const levels = useMemo(() => {
    return Array.from(nodeState.levels.entries())
      .sort(([a], [b]) => a - b)
      .map(([level, state]) => ({
        level,
        slots: Array.from(state.slots.values()).sort((a, b) => a.slotId - b.slotId),
      }));
  }, [nodeState.levels]);

  const handleSlotClick = (level: number, slot: SlotState) => {
    if (selectedSlot?.level === level && selectedSlot?.slotId === slot.slotId) {
      setSelectedSlot(null);
    } else {
      setSelectedSlot({
        level,
        slotId: slot.slotId,
        heat: slot.heat,
        k: slot.k,
      });
    }
    onSlotClick?.(level, slot.slotId);
  };

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
          selectedSlotId={selectedSlot?.level === level ? selectedSlot.slotId : null}
          onSlotHover={onSlotHover}
          onSlotClick={(slot) => handleSlotClick(level, slot)}
        />
      ))}

      {/* Slot detail panel */}
      <AnimatePresence>
        {selectedSlot && (
          <SlotDetailPanel
            nodeId={nodeState.nodeId}
            level={selectedSlot.level}
            slotId={selectedSlot.slotId}
            heat={selectedSlot.heat}
            k={selectedSlot.k}
            onClose={() => setSelectedSlot(null)}
          />
        )}
      </AnimatePresence>

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
  selectedSlotId: number | null;
  onSlotHover?: (level: number, slot: number, heat: number) => void;
  onSlotClick: (slot: SlotState) => void;
}

function LevelRow({ level, slots, selectedSlotId, onSlotHover, onSlotClick }: LevelRowProps) {
  const levelColors = [
    "border-red-500/50",
    "border-orange-500/50",
    "border-amber-500/50",
    "border-yellow-500/50",
    "border-green-500/50",
    "border-teal-500/50",
    "border-blue-500/50",
  ];

  // Calculate level stats
  const stats = useMemo(() => {
    if (slots.length === 0) return null;
    const heats = slots.map((s) => s.heat);
    const avgHeat = heats.reduce((a, b) => a + b, 0) / heats.length;
    const maxHeat = Math.max(...heats);
    const hotSlots = heats.filter((h) => h > 0.7).length;
    return { avgHeat, maxHeat, hotSlots };
  }, [slots]);

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
            isSelected={selectedSlotId === slot.slotId}
            onHover={onSlotHover}
            onClick={() => onSlotClick(slot)}
          />
        ))}
      </div>

      {/* Level stats */}
      <div className="flex w-32 items-center gap-2 text-right text-xs text-muted-foreground">
        {stats && (
          <>
            <span className="hidden sm:inline">
              avg: {(stats.avgHeat * 100).toFixed(0)}%
            </span>
            {stats.hotSlots > 0 && (
              <span className="text-amber-500">{stats.hotSlots} hot</span>
            )}
          </>
        )}
        <span>{slots.length} slots</span>
      </div>
    </div>
  );
}

interface SlotCellProps {
  slot: SlotState;
  level: number;
  isSelected: boolean;
  onHover?: (level: number, slot: number, heat: number) => void;
  onClick: () => void;
}

function SlotCell({ slot, level, isSelected, onHover, onClick }: SlotCellProps) {
  const bgColor = heatToColor(slot.heat);
  const isHot = slot.heat > 0.7;

  return (
    <motion.div
      className={cn(
        "relative h-6 w-6 cursor-pointer rounded-sm transition-all",
        "hover:scale-110 hover:ring-2 hover:ring-primary/50",
        isHot && "animate-pulse-subtle",
        isSelected && "ring-2 ring-primary scale-110"
      )}
      style={{ backgroundColor: bgColor }}
      initial={{ opacity: 0, scale: 0.8 }}
      animate={{ opacity: 1, scale: isSelected ? 1.1 : 1 }}
      transition={{ duration: 0.2 }}
      onMouseEnter={() => onHover?.(level, slot.slotId, slot.heat)}
      onClick={onClick}
      title={`Slot ${slot.slotId}: heat=${(slot.heat * 100).toFixed(0)}%, k=${slot.k}`}
    >
      {isSelected && (
        <motion.div
          className="absolute inset-0 rounded-sm ring-2 ring-primary"
          layoutId="slot-selection"
        />
      )}
    </motion.div>
  );
}

interface SlotDetailPanelProps {
  nodeId: number;
  level: number;
  slotId: number;
  heat: number;
  k: number;
  onClose: () => void;
}

function SlotDetailPanel({ nodeId, level, slotId, heat, k, onClose }: SlotDetailPanelProps) {
  const heatHistory = useSlotHeatHistory(nodeId, level, slotId, 60);

  const stats = useMemo(() => {
    if (heatHistory.length === 0) return null;
    const values = heatHistory.map((p) => p.value);
    return {
      min: Math.min(...values),
      max: Math.max(...values),
      avg: values.reduce((a, b) => a + b, 0) / values.length,
    };
  }, [heatHistory]);

  const sparklineData = useMemo(() => {
    return heatHistory.map((p) => ({
      timestamp: p.timestamp,
      value: p.value,
    }));
  }, [heatHistory]);

  return (
    <motion.div
      initial={{ height: 0, opacity: 0 }}
      animate={{ height: "auto", opacity: 1 }}
      exit={{ height: 0, opacity: 0 }}
      transition={{ duration: 0.2 }}
      className="overflow-hidden"
    >
      <div className="mt-4 rounded-lg border border-border bg-card p-4">
        <div className="flex items-start justify-between">
          <div>
            <h4 className="font-medium text-foreground">
              Slot {slotId} (Level {level})
            </h4>
            <p className="text-sm text-muted-foreground">
              k={k}, current heat: {(heat * 100).toFixed(0)}%
            </p>
          </div>
          <button
            onClick={onClose}
            className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Sparkline */}
        <div className="mt-4">
          <div className="flex items-center justify-between text-xs text-muted-foreground mb-1">
            <span>Heat Trend (last hour)</span>
            <span>{heatHistory.length} samples</span>
          </div>
          {sparklineData.length > 1 ? (
            <Sparkline
              data={sparklineData}
              width={300}
              height={48}
              color={heatToColor(heat)}
              thresholdValue={0.7}
              thresholdColor="hsl(var(--warning))"
              showArea
              className="w-full"
            />
          ) : (
            <div className="flex h-12 items-center justify-center rounded bg-muted/30 text-xs text-muted-foreground">
              Collecting data...
            </div>
          )}
        </div>

        {/* Stats */}
        {stats && (
          <div className="mt-4 grid grid-cols-3 gap-2">
            <StatBox label="Min (1h)" value={`${(stats.min * 100).toFixed(0)}%`} />
            <StatBox label="Max (1h)" value={`${(stats.max * 100).toFixed(0)}%`} status={stats.max > 0.7 ? "warning" : undefined} />
            <StatBox label="Avg (1h)" value={`${(stats.avg * 100).toFixed(0)}%`} />
          </div>
        )}
      </div>
    </motion.div>
  );
}

function StatBox({ label, value, status }: { label: string; value: string; status?: "warning" | "critical" }) {
  return (
    <div className="rounded bg-muted/50 px-2 py-1.5 text-center">
      <p className="text-xs text-muted-foreground">{label}</p>
      <p
        className={cn(
          "font-medium",
          status === "warning" && "text-amber-500",
          status === "critical" && "text-red-500",
          !status && "text-foreground"
        )}
      >
        {value}
      </p>
    </div>
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
