import type { NormalizedAgentEvent } from "@mars/shared-types";
import { CheckCircle2Icon, CircleAlertIcon, FileCode2Icon, TerminalIcon, WrenchIcon } from "lucide-react";

import { Badge } from "@/components/ui/badge";

export function AgentTimeline({ events }: { events: NormalizedAgentEvent[] }) {
  return (
    <section className="grid gap-2" aria-label="Agent timeline" aria-live="polite">
      {events.map((event, index) => (
        <article className="rounded-lg border bg-muted/20 p-2" key={`${event.type}-${index}`}>
          <EventBody event={event} />
        </article>
      ))}
      {events.length === 0 ? <p className="py-4 text-center text-xs text-muted-foreground">Agent events will appear here.</p> : null}
    </section>
  );
}

function EventBody({ event }: { event: NormalizedAgentEvent }) {
  switch (event.type) {
    case "checkpoint":
      return <Row icon={CheckCircle2Icon} label="Session checkpoint" detail={event.sessionId} />;
    case "text":
      return <p className="whitespace-pre-wrap text-xs leading-relaxed">{event.text}</p>;
    case "command":
      return <Row icon={TerminalIcon} label={`Command · ${event.status}`} detail={event.command} />;
    case "test":
      return <Row icon={CheckCircle2Icon} label={`Test · ${event.status}`} detail={event.command} />;
    case "file_change":
      return <Row icon={FileCode2Icon} label={`File · ${event.change}`} detail={event.path} />;
    case "tool":
      return <Row icon={WrenchIcon} label="Tool" detail={event.name} />;
    case "permission":
      return <Row icon={CircleAlertIcon} label={`Permission · ${event.tool}`} detail={event.description} />;
    case "unknown":
      return <Row icon={CircleAlertIcon} label="Provider event" detail={event.providerType} />;
    case "end":
      return (
        <div className="flex items-center justify-between gap-2">
          <span className="text-xs">{event.message ?? (event.success ? "Agent completed" : "Agent stopped")}</span>
          <Badge variant={event.success ? "default" : "destructive"}>{event.success ? "complete" : "failed"}</Badge>
        </div>
      );
  }
}

function Row({ detail, icon: Icon, label }: { detail: string; icon: typeof TerminalIcon; label: string }) {
  return (
    <div className="flex items-start gap-2">
      <Icon className="mt-0.5 size-3.5 text-primary" />
      <div className="min-w-0">
        <p className="text-[10px] uppercase tracking-wider text-muted-foreground">{label}</p>
        <p className="break-all font-mono text-xs">{detail}</p>
      </div>
    </div>
  );
}
