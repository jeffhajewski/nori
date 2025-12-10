"use client";

import { useEffect, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { useEventStore } from "@/stores/eventStore";
import { formatTimestamp } from "@/lib/utils";
import type { WireSwimEvt } from "@/types/events";

interface SwimEvent {
  id: string;
  timestamp: number;
  node: number;
  kind: string;
}

const MAX_EVENTS = 30;

export function SwimTimeline({ filterNode }: { filterNode?: number | null }) {
  const [events, setEvents] = useState<SwimEvent[]>([]);

  // Subscribe to SWIM events
  useEffect(() => {
    const unsub = useEventStore.subscribe(
      (state) => state.eventBuffer,
      (buffer) => {
        if (buffer.length === 0) return;

        const latest = buffer[buffer.length - 1];
        const newEvents: SwimEvent[] = [];

        for (const evt of latest.events) {
          if (evt.event.type === "Swim") {
            const swimEvt = evt.event.data;
            newEvents.push({
              id: `${evt.ts}-${swimEvt.node}-${Math.random()}`,
              timestamp: evt.ts,
              node: swimEvt.node,
              kind: swimEvt.kind,
            });
          }
        }

        if (newEvents.length > 0) {
          setEvents((prev) => [...newEvents, ...prev].slice(0, MAX_EVENTS));
        }
      }
    );

    return unsub;
  }, []);

  const filteredEvents = filterNode !== null && filterNode !== undefined
    ? events.filter((e) => e.node === filterNode)
    : events;

  return (
    <div className="space-y-2">
      {filteredEvents.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          No SWIM events yet. Waiting for membership updates...
        </p>
      ) : (
        <div className="max-h-80 space-y-1 overflow-y-auto pr-2">
          <AnimatePresence mode="popLayout">
            {filteredEvents.map((event) => (
              <SwimEventRow key={event.id} event={event} />
            ))}
          </AnimatePresence>
        </div>
      )}
    </div>
  );
}

function SwimEventRow({ event }: { event: SwimEvent }) {
  const getEventConfig = (kind: string) => {
    switch (kind) {
      case "Alive":
        return {
          icon: (
            <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4.318 6.318a4.5 4.5 0 000 6.364L12 20.364l7.682-7.682a4.5 4.5 0 00-6.364-6.364L12 7.636l-1.318-1.318a4.5 4.5 0 00-6.364 0z" />
            </svg>
          ),
          bg: "bg-status-healthy/20",
          color: "text-status-healthy",
          label: "Alive",
        };
      case "Suspect":
        return {
          icon: (
            <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
            </svg>
          ),
          bg: "bg-status-warning/20",
          color: "text-status-warning",
          label: "Suspect",
        };
      case "Confirm":
        return {
          icon: (
            <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
          ),
          bg: "bg-status-critical/20",
          color: "text-status-critical",
          label: "Confirmed Dead",
        };
      case "Leave":
        return {
          icon: (
            <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M17 16l4-4m0 0l-4-4m4 4H7m6 4v1a3 3 0 01-3 3H6a3 3 0 01-3-3V7a3 3 0 013-3h4a3 3 0 013 3v1" />
            </svg>
          ),
          bg: "bg-muted",
          color: "text-muted-foreground",
          label: "Left Cluster",
        };
      default:
        return {
          icon: (
            <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
          ),
          bg: "bg-muted",
          color: "text-muted-foreground",
          label: kind,
        };
    }
  };

  const config = getEventConfig(event.kind);

  return (
    <motion.div
      layout
      initial={{ opacity: 0, x: -20 }}
      animate={{ opacity: 1, x: 0 }}
      exit={{ opacity: 0, x: 20 }}
      className="flex items-center gap-3 rounded-lg bg-muted/30 px-3 py-2"
    >
      <div className={`flex h-6 w-6 items-center justify-center rounded-full ${config.bg} ${config.color}`}>
        {config.icon}
      </div>
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <span className="font-medium text-foreground">Node {event.node}</span>
          <span className={`text-xs ${config.color}`}>{config.label}</span>
        </div>
      </div>
      <span className="text-xs text-muted-foreground whitespace-nowrap">
        {formatTimestamp(event.timestamp)}
      </span>
    </motion.div>
  );
}
