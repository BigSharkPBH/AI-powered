import { FormEvent, useCallback, useEffect, useRef, useState } from "react";
import { ArrowUp, Bot, ChevronDown, FileText, Hand, MessageSquare, MicOff, Play, RotateCcw, Square, Volume2, Wrench } from "lucide-react";

import * as api from "../../api/commands";
import "../../styles/workspace.css";
import { connectLiveKitRoom, disconnectLiveKitRoom } from "./livekit-room";
import type {
  AgentCommandInput,
  CommandResult,
  RuntimeStatus,
  SessionReplyEvent,
  SessionTranscriptEvent,
} from "../../generated/bindings";

const errorText = (error: { code: string; message: string; field?: string | null }) =>
  `${error.field ? error.field + "：" : ""}${error.code}：${error.message}`;

const ACTIVE_PHASES = new Set([
  "preparing",
  "listening",
  "thinking",
  "speaking",
  "stopping",
  "recovering",
  "blocked",
]);

const PHASE_LABELS: Record<string, string> = {
  idle: "未开始",
  preparing: "准备中",
  listening: "聆听中",
  thinking: "思考中",
  speaking: "回复中",
  stopping: "停止中",
  recovering: "恢复中",
  blocked: "需要处理",
  completed: "已结束",
  failed: "会话异常",
};

const MODE_LABELS: Record<string, string> = {
  ai_active: "AI 应答",
  operator_speaking: "人工接管",
  paused: "已暂停",
  muted: "已静音",
};

export type SessionListen = <T>(
  event: string,
  handler: (payload: T) => void,
) => Promise<() => void> | (() => void);

export interface WorkspaceSessionProps {
  finalizeUtterance?: (text: string) => Promise<void>;
  listen?: SessionListen;
}

async function defaultFinalizeUtterance(text: string) {
  const result = await api.finalizeSessionUtterance(text);
  if (!result.ok) {
    throw new Error(errorText(result.error));
  }
}

async function defaultListen<T>(event: string, handler: (payload: T) => void) {
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) {
    return () => {};
  }
  try {
    const mod = await import("@tauri-apps/api/event");
    return mod.listen<T>(event, (envelope) => handler(envelope.payload));
  } catch {
    return () => {};
  }
}

export function WorkspaceSession({
  finalizeUtterance = defaultFinalizeUtterance,
  listen = defaultListen,
}: WorkspaceSessionProps) {
  const [phase, setPhase] = useState("idle");
  const [mode, setMode] = useState("ai_active");
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [transcript, setTranscript] = useState("");
  const [reply, setReply] = useState("");
  const [unusedMaterials, setUnusedMaterials] = useState(false);
  const [message, setMessage] = useState("正在读取会话状态…");
  const [busy, setBusy] = useState(false);
  const [utterance, setUtterance] = useState("");
  const [sayText, setSayText] = useState("");
  const [correctText, setCorrectText] = useState("");
  const [revision, setRevision] = useState(0);
  const [reportSummary, setReportSummary] = useState("");
  const [reportDetail, setReportDetail] = useState("");
  const [transport, setTransport] = useState<"direct" | "livekit">("direct");
  const [livekitState, setLivekitState] = useState("idle");
  const livekitRoom = useRef<Awaited<ReturnType<typeof connectLiveKitRoom>> | null>(null);
  const statusSeq = useRef(0);
  const transcriptSeq = useRef(0);
  const replySeq = useRef(0);
  const sessionIdRef = useRef<string | null>(null);

  const applyStatus = useCallback((next: RuntimeStatus) => {
    if (next.seq <= statusSeq.current) return;
    statusSeq.current = next.seq;
    setPhase(next.phase);
    setMode(next.mode);
    setUnusedMaterials(next.unusedMaterials);
    setRevision(next.revision);
    if (next.lastErrorCode) {
      setMessage(`runtime：${next.lastErrorCode}：会话运行时错误`);
    }
  }, []);

  const applyTranscript = useCallback((payload: SessionTranscriptEvent) => {
    if (payload.seq <= transcriptSeq.current) return;
    transcriptSeq.current = payload.seq;
    setTranscript(payload.text);
  }, []);

  const applyReply = useCallback((payload: SessionReplyEvent) => {
    if (payload.seq <= replySeq.current) return;
    replySeq.current = payload.seq;
    setReply(payload.text);
  }, []);

  const refresh = useCallback(
    async (id?: string | null) => {
      const target = id ?? sessionIdRef.current;
      try {
        const statusResult = await api.getRuntimeStatus();
        if (statusResult.ok) {
          applyStatus(statusResult.data);
        } else {
          setMessage(errorText(statusResult.error));
        }
        if (target) {
          const detail = await api.getSession(target);
          if (detail.ok) {
            const last = detail.data.turns.at(-1);
            if (last) {
              setTranscript(last.userText);
              setReply(last.assistantText);
              setUnusedMaterials(!last.materialsUsed);
            }
          } else {
            setMessage(errorText(detail.error));
          }
        }
      } catch {
        setMessage("IPC_UNAVAILABLE：无法读取会话状态");
      }
    },
    [applyStatus],
  );

  useEffect(() => {
    sessionIdRef.current = sessionId;
  }, [sessionId]);

  useEffect(() => {
    void (async () => {
      try {
        const result = await api.getRuntimeStatus();
        if (result.ok) {
          applyStatus(result.data);
          setMessage("");
        } else {
          setMessage(errorText(result.error));
        }
      } catch {
        setMessage("IPC_UNAVAILABLE：无法读取会话状态");
      }
    })();
  }, [applyStatus]);

  useEffect(() => {
    let cancelled = false;
    const unlisteners: Array<() => void> = [];
    void (async () => {
      const topics: Array<[string, (payload: never) => void]> = [
        ["runtime.status.v1", applyStatus as (payload: never) => void],
        ["session.transcript.v1", applyTranscript as (payload: never) => void],
        ["session.reply.v1", applyReply as (payload: never) => void],
      ];
      for (const [event, handler] of topics) {
        try {
          const unlisten = await Promise.resolve(listen(event, handler));
          if (cancelled) {
            unlisten();
            return;
          }
          unlisteners.push(unlisten);
        } catch {
          // Event bus is optional when IPC is unavailable.
        }
      }
    })();
    return () => {
      cancelled = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, [listen, applyStatus, applyTranscript, applyReply]);

  async function run(action: () => Promise<CommandResult<unknown>>, success = "") {
    setBusy(true);
    try {
      const result = await action();
      if (!result.ok) {
        setMessage(errorText(result.error));
        return false;
      }
      if (success) setMessage(success);
      else setMessage("");
      return true;
    } catch {
      setMessage("IPC_UNAVAILABLE：本地操作失败");
      return false;
    } finally {
      setBusy(false);
    }
  }

  async function start() {
    setBusy(true);
    try {
      const result = await api.startSession(transport);
      if (!result.ok) {
        setMessage(errorText(result.error));
        return;
      }
      if (result.data.kind === "blocked") {
        setMessage(
          result.data.issues.map((issue) => `${issue.area}：${issue.code}：${issue.action}`).join("；"),
        );
        return;
      }
      setSessionId(result.data.session.id);
      sessionIdRef.current = result.data.session.id;
      setPhase(result.data.session.status);
      setTranscript("");
      setReply("");
      setUnusedMaterials(false);
      setUtterance("");
      setSayText("");
      setCorrectText("");
      setReportSummary("");
      setReportDetail("");
      setMessage("");
      setLivekitState("idle");
      if (result.data.livekit) {
        try {
          livekitRoom.current = await connectLiveKitRoom(result.data.livekit);
          setLivekitState("connected");
        } catch {
          setLivekitState("error");
          setMessage("LIVEKIT_CONNECT_FAILED：无法进入房间");
        }
      }
      await refresh(result.data.session.id);
    } catch {
      setMessage("IPC_UNAVAILABLE：本地操作失败");
    } finally {
      setBusy(false);
    }
  }

  async function stop() {
    await disconnectLiveKitRoom(livekitRoom.current);
    livekitRoom.current = null;
    setLivekitState("idle");
    const ok = await run(() => api.stopSession());
    if (ok) {
      await refresh();
    }
  }

  async function setModeName(next: "ai_active" | "operator_speaking" | "paused" | "muted") {
    const ok = await run(() => api.setSessionMode(next));
    if (ok) {
      setMode(next);
      await refresh();
    }
  }

  function commandId() {
    return crypto.randomUUID();
  }

  function reportLines(result: Record<string, unknown>) {
    const report = result.report;
    if (!report || typeof report !== "object") return "";
    const parts: string[] = [];
    const record = report as Record<string, unknown>;
    for (const key of ["strengths", "followUps", "limitations"] as const) {
      const value = record[key];
      if (Array.isArray(value)) {
        for (const item of value) {
          if (typeof item === "string" && item.trim()) parts.push(item);
        }
      }
    }
    return parts.join("；");
  }

  async function runAgentCommand(input: AgentCommandInput) {
    setBusy(true);
    try {
      const result = await api.sessionAgentCommand(input);
      if (!result.ok) {
        setMessage(errorText(result.error));
        return;
      }
      if (!result.data.ok) {
        setMessage(result.data.error);
        return;
      }
      setMessage("");
      if (input.action === "report") {
        const summary =
          typeof result.data.result.summary === "string" ? result.data.result.summary : "";
        setReportSummary(summary);
        setReportDetail(reportLines(result.data.result));
      }
      await refresh();
    } catch {
      setMessage("IPC_UNAVAILABLE：本地操作失败");
    } finally {
      setBusy(false);
    }
  }

  async function submitSay(event: FormEvent) {
    event.preventDefault();
    await runAgentCommand({
      id: commandId(),
      action: "say",
      text: sayText.trim() || null,
      answer: null,
      mode: null,
      expectedRevision: revision,
    });
  }

  async function submitCorrect() {
    await runAgentCommand({
      id: commandId(),
      action: "correct",
      text: null,
      answer: correctText.trim() || null,
      mode: null,
      expectedRevision: revision,
    });
  }

  async function submitRetry() {
    await runAgentCommand({
      id: commandId(),
      action: "retry",
      text: null,
      answer: null,
      mode: null,
      expectedRevision: revision,
    });
  }

  async function submitReport() {
    await runAgentCommand({
      id: commandId(),
      action: "report",
      text: null,
      answer: null,
      mode: null,
      expectedRevision: revision,
    });
  }

  async function submitFinalize(event: FormEvent) {
    event.preventDefault();
    if (!ACTIVE_PHASES.has(phase)) {
      setMessage("还没有开始会话，请先点开始");
      return;
    }
    setBusy(true);
    try {
      await finalizeUtterance(utterance.trim());
      setMessage("");
      await refresh();
    } catch (error) {
      const text = error instanceof Error ? error.message : "";
      setMessage(text.includes("：") ? text : "IPC_UNAVAILABLE：本地操作失败");
    } finally {
      setBusy(false);
    }
  }

  const active = ACTIVE_PHASES.has(phase);

  return (
    <section className="workspace-session" aria-labelledby="workspace-session-heading">
      <header className="session-toolbar">
        <div className="session-toolbar-meta">
          <h2 id="workspace-session-heading">当前会话</h2>
          <span className="status-badge" data-active={active}>
            {PHASE_LABELS[phase] ?? phase}
          </span>
          <span className="session-mode">{MODE_LABELS[mode] ?? mode}</span>
          {livekitState !== "idle" && (
            <span className="session-mode">LiveKit {livekitState === "connected" ? "已连接" : "连接失败"}</span>
          )}
        </div>
        <div className="session-toolbar-controls">
          <fieldset className="session-transport">
            <legend>传输方式</legend>
            <label>
              <input
                type="radio"
                name="transport"
                value="direct"
                checked={transport === "direct"}
                disabled={busy || active}
                onChange={() => setTransport("direct")}
              />
              <span>本机直连</span>
            </label>
            <label>
              <input
                type="radio"
                name="transport"
                value="livekit"
                checked={transport === "livekit"}
                disabled={busy || active}
                onChange={() => setTransport("livekit")}
              />
              <span>LiveKit</span>
            </label>
          </fieldset>
          <div className="service-actions session-controls">
            <button className="button-primary" disabled={busy || active} type="button" onClick={() => void start()}>
              <Play size={14} aria-hidden="true" />开始会话
            </button>
            <button disabled={!active} type="button" onClick={() => void stop()}>
              <Square size={14} aria-hidden="true" />停止
            </button>
            <button disabled={!active} type="button" onClick={() => void setModeName("operator_speaking")}>
              <Hand size={14} aria-hidden="true" />接管
            </button>
            <button disabled={busy || !active} type="button" onClick={() => void setModeName("ai_active")}>
              <Bot size={14} aria-hidden="true" />恢复 AI
            </button>
            <button disabled={busy || !active} type="button" onClick={() => void setModeName("muted")}>
              <MicOff size={14} aria-hidden="true" />静音
            </button>
          </div>
        </div>
      </header>
      {message && (
        <p className="services-message session-message" role="status">
          {message}
        </p>
      )}
      <div className="session-conversation" role="region" aria-label="当前轮对话" tabIndex={0}>
        {!transcript && !reply ? (
          <div className="session-welcome">
            <span className="session-welcome-icon"><MessageSquare size={25} strokeWidth={1.5} aria-hidden="true" /></span>
            <h3>{active ? "正在等待你的输入" : "开始一段新对话"}</h3>
            <p>{active ? "说出问题，或在下方输入语句。" : "点击「开始会话」，与 AI 虚拟助手交流。"}</p>
          </div>
        ) : (
          <div className="session-turn">
            <p className="session-turn-label">当前轮</p>
            {transcript && (
              <article className="session-bubble session-bubble-user" aria-label="用户转写">
                <h3>你 <span>· 转写</span></h3>
                <p>{transcript}</p>
              </article>
            )}
            {reply && (
              <article className="session-bubble session-bubble-assistant" aria-label="AI 回复">
                <h3><Bot size={16} aria-hidden="true" />AI 虚拟助手</h3>
                <p>{reply}</p>
              </article>
            )}
          </div>
        )}
        {unusedMaterials && <p className="session-materials-note">本轮未使用资料</p>}
      </div>
      <form className="session-compose" onSubmit={submitFinalize}>
        <label htmlFor="session-utterance">语句输入</label>
        <div className="session-compose-row">
          <input
            id="session-utterance"
            value={utterance}
            placeholder={active ? "输入你想说的话…" : "开始会话后发送语句…"}
            onChange={(event) => setUtterance(event.target.value)}
          />
          <button className="button-primary" disabled={busy || !active} type="submit">
            <ArrowUp size={16} aria-hidden="true" />发送
          </button>
        </div>
      </form>
      <details className="session-tools">
        <summary><Wrench size={15} aria-hidden="true" />会话工具<ChevronDown size={15} className="session-tools-chevron" aria-hidden="true" /></summary>
        <div className="session-tools-body" role="region" aria-label="会话工具">
          <form className="service-form session-tool-form" onSubmit={submitSay}>
            <label htmlFor="session-say">朗读文本</label>
            <div className="session-tool-row">
              <input id="session-say" value={sayText} onChange={(event) => setSayText(event.target.value)} placeholder="输入需要 AI 朗读的文本" />
              <button disabled={busy || !active} type="submit"><Volume2 size={15} aria-hidden="true" />朗读</button>
            </div>
          </form>
          <form className="service-form session-tool-form" onSubmit={(event) => { event.preventDefault(); void submitCorrect(); }}>
            <label htmlFor="session-correct">纠正内容</label>
            <div className="session-tool-row">
              <input id="session-correct" value={correctText} onChange={(event) => setCorrectText(event.target.value)} placeholder="输入修正后的回答" />
              <button disabled={busy || !active} type="submit">纠正</button>
            </div>
          </form>
          <div className="service-actions">
            <button disabled={busy || !active} type="button" onClick={() => void submitRetry()}><RotateCcw size={15} aria-hidden="true" />重试</button>
            <button disabled={busy || !active} type="button" onClick={() => void submitReport()}><FileText size={15} aria-hidden="true" />报告</button>
          </div>
          {(reportSummary || reportDetail) && (
            <section className="session-report" aria-labelledby="session-report-heading">
              <h3 id="session-report-heading">会话纪要</h3>
              {reportSummary && <p>{reportSummary}</p>}
              {reportDetail && <p className="muted">{reportDetail}</p>}
            </section>
          )}
        </div>
      </details>
    </section>
  );
}
