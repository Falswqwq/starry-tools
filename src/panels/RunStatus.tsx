/** 底部那颗状态药丸。跑完工作流后点开就是运行记录。 */

import { ChevronUp, FolderOpen, Trash2 } from 'lucide-react';
import { useState } from 'react';

import { useStore } from '../state/store';
import { Button } from '../ui/Button';
import { Popover, PopoverContent, PopoverTrigger } from '../ui/Popover';

const STATUS_TEXT = { ok: '完成', failed: '出错', skipped: '跳过' } as const;

export function RunStatus() {
  const report = useStore((state) => state.report);
  const status = useStore((state) => state.status);
  const running = useStore((state) => state.running);
  const select = useStore((state) => state.select);
  const reveal = useStore((state) => state.reveal);
  const clearReport = useStore((state) => state.clearReport);

  const [open, setOpen] = useState(false);

  const failures = report?.nodes.filter((node) => node.status !== 'ok').length ?? 0;
  const kind = status?.kind ?? 'idle';

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        if (!report) return;
        setOpen(next);
      }}
    >
      <PopoverTrigger asChild>
        <button type="button" className="pill" data-kind={kind} disabled={!report}>
          <span className={`pill__dot ${running ? 'is-busy' : ''}`} aria-hidden="true" />
          <span className="pill__text">
            {status?.text ?? (running ? '正在运行…' : '就绪')}
          </span>
          {report && (
            <span className="pill__stat">
              {report.nodes.length} 节点 · {report.durationMs}ms
              {failures > 0 && ` · ${failures} 未通过`}
            </span>
          )}
          {report && <ChevronUp size={12} className="pill__caret" aria-hidden="true" />}
        </button>
      </PopoverTrigger>

      {report && (
        <PopoverContent className="log" side="top" align="center" sideOffset={10}>
          <header className="popover__head">
            <h2 className="popover__title">运行记录</h2>
            <span className="popover__meta">{report.nodes.length} 个节点</span>
            <span className="popover__spacer" />
            {report.outputDir && (
              <Button
                size="icon-sm"
                variant="ghost"
                tooltip="打开输出目录"
                onClick={() => void reveal(report.outputDir ?? '')}
              >
                <FolderOpen size={12} />
              </Button>
            )}
            <Button
              size="icon-sm"
              variant="ghost"
              tooltip="清除记录"
              onClick={() => {
                setOpen(false);
                clearReport();
              }}
            >
              <Trash2 size={12} />
            </Button>
          </header>

          <div className="log__body">
            {report.nodes.length === 0 ? (
              <ul className="notes notes--bad">
                {report.issues.map((issue) => (
                  <li key={issue.message}>{issue.message}</li>
                ))}
              </ul>
            ) : (
              <ol className="log__rows">
                {report.nodes.map((node, index) => (
                  <li key={node.nodeId} className={`log__row log__row--${node.status}`}>
                    <span className="log__index">{String(index + 1).padStart(2, '0')}</span>
                    <button
                      type="button"
                      className="log__name"
                      title="在画布上选中这个节点"
                      onClick={() => select(node.nodeId)}
                    >
                      {node.name}
                    </button>
                    <span className={`log__status log__status--${node.status}`}>
                      {STATUS_TEXT[node.status]}
                    </span>
                    <span className="log__ms">{node.elapsedMs}ms</span>
                    <span className="log__detail">
                      {node.error && <span className="log__msg">{node.error}</span>}
                      {node.warnings.map((warning) => (
                        <span className="log__warn" key={warning}>
                          {warning}
                        </span>
                      ))}
                      {node.outputs.map((output) => (
                        <span className="log__out" key={output.portId}>
                          {output.summary}
                        </span>
                      ))}
                    </span>
                  </li>
                ))}
              </ol>
            )}
          </div>
        </PopoverContent>
      )}
    </Popover>
  );
}
