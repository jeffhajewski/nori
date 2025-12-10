"use client";

import { useEffect, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { useEventStore } from "@/stores/eventStore";
import { formatTimestamp } from "@/lib/utils";
import type { WireRaftEvt } from "@/types/events";

interface RaftEvent {
  id: string;
  timestamp: number;
  shard: number;
  term: number;
  kind: string;
  node?: number;
  from?: number;
}

const MAX_EVENTS = 50;

export function ElectionTimeline({ filterShard }: { filterShard?: number | null }) {
  const [events, setEvents] = useState<RaftEvent[]>([]);

  // Subscribe to Raft events
  useEffect(() => {
    const unsub = useEventStore.subscribe(
      (state) => state.eventBuffer,
      (buffer) => {
        if (buffer.length === 0) return;

        const latest = buffer[buffer.length - 1];
        const newEvents: RaftEvent[] = [];

        for (const evt of latest.events) {
          if (evt.event.type === "Raft") {
            const raftEvt = evt.event.data;
            newEvents.push({
              id: `${evt.ts}-${raftEvt.shard}-${raftEvt.term}-${Math.random()}`,
              timestamp: evt.ts,
              shard: raftEvt.shard,
              term: raftEvt.term,
              kind: raftEvt.kind.kind,
              node: raftEvt.kind.kind === "LeaderElected" ? raftEvt.kind.node : undefined,
              from: raftEvt.kind.kind === "VoteReq" || raftEvt.kind.kind === "VoteGranted"
                ? raftEvt.kind.from
                : undefined,
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

  const filteredEvents = filterShard !== null && filterShard !== undefined
    ? events.filter((e) => e.shard === filterShard)
    : events;

  return (
    <div className="space-y-2">
      {filteredEvents.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          No Raft events yet. Waiting for elections...
        </p>
      ) : (
        <div className="max-h-96 space-y-1 overflow-y-auto pr-2">
          <AnimatePresence mode="popLayout">
            {filteredEvents.map((event) => (
              <EventRow key={event.id} event={event} />
            ))}
          </AnimatePresence>
        </div>
      )}
    </div>
  );
}

function EventRow({ event }: { event: RaftEvent }) {
  const getEventIcon = (kind: string) => {
    switch (kind) {
      case "LeaderElected":
        return (
          <div className="flex h-6 w-6 items-center justify-center rounded-full bg-status-healthy/20 text-status-healthy">
            <svg className="h-3.5 w-3.5" fill="currentColor" viewBox="0 0 20 20">
              <path d="M9.049 2.927c.3-.921 1.603-.921 1.902 0l1.07 3.292a1 1 0 00.95.69h3.462c.969 0 1.371 1.24.588 1.81l-2.8 2.034a1 1 0 00-.364 1.118l1.07 3.292c.3.921-.755 1.688-1.54 1.118l-2.8-2.034a1 1 0 00-1.175 0l-2.8 2.034c-.784.57-1.838-.197-1.539-1.118l1.07-3.292a1 1 0 00-.364-1.118L2.98 8.72c-.783-.57-.38-1.81.588-1.81h3.461a1 1 0 00.951-.69l1.07-3.292z" />
            </svg>
          </div>
        );
      case "VoteReq":
        return (
          <div className="flex h-6 w-6 items-center justify-center rounded-full bg-blue-500/20 text-blue-500">
            <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8.228 9c.549-1.165 2.03-2 3.772-2 2.21 0 4 1.343 4 3 0 1.4-1.278 2.575-3.006 2.907-.542.104-.994.54-.994 1.093m0 3h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
          </div>
        );
      case "VoteGranted":
        return (
          <div className="flex h-6 w-6 items-center justify-center rounded-full bg-green-500/20 text-green-500">
            <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
            </svg>
          </div>
        );
      case "StepDown":
        return (
          <div className="flex h-6 w-6 items-center justify-center rounded-full bg-status-warning/20 text-status-warning">
            <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 14l-7 7m0 0l-7-7m7 7V3" />
            </svg>
          </div>
        );
      default:
        return (
          <div className="flex h-6 w-6 items-center justify-center rounded-full bg-muted text-muted-foreground">
            <svg className="h-3.5 w-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
          </div>
        );
    }
  };

  const getEventDescription = (event: RaftEvent) => {
    switch (event.kind) {
      case "LeaderElected":
        return (
          <span>
            <strong className="text-status-healthy">Node {event.node}</strong> elected leader
          </span>
        );
      case "VoteReq":
        return (
          <span>
            Vote requested from <strong>Node {event.from}</strong>
          </span>
        );
      case "VoteGranted":
        return (
          <span>
            Vote granted to <strong>Node {event.from}</strong>
          </span>
        );
      case "StepDown":
        return <span>Leader stepped down</span>;
      default:
        return <span>{event.kind}</span>;
    }
  };

  return (
    <motion.div
      layout
      initial={{ opacity: 0, x: -20 }}
      animate={{ opacity: 1, x: 0 }}
      exit={{ opacity: 0, x: 20 }}
      className="flex items-center gap-3 rounded-lg bg-muted/30 px-3 py-2"
    >
      {getEventIcon(event.kind)}
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <span className="rounded bg-primary/20 px-1.5 py-0.5 text-xs font-medium text-primary">
            Shard {event.shard}
          </span>
          <span className="text-xs text-muted-foreground">Term {event.term}</span>
        </div>
        <p className="mt-0.5 text-sm text-foreground truncate">
          {getEventDescription(event)}
        </p>
      </div>
      <span className="text-xs text-muted-foreground whitespace-nowrap">
        {formatTimestamp(event.timestamp)}
      </span>
    </motion.div>
  );
}
