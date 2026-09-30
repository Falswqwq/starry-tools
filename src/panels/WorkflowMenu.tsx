/** 加载已保存的工作流。只干这一件事 —— 改名和写说明在画布左上角。 */

import { FolderOpen, Plus, Trash2, Undo2 } from 'lucide-react';
import { useState } from 'react';

import { formatTime } from '../lib/graph';
import { useStore } from '../state/store';
import { Button } from '../ui/Button';
import { Popover, PopoverContent, PopoverTrigger } from '../ui/Popover';

export function WorkflowMenu() {
  const meta = useStore((state) => state.meta);
  const summaries = useStore((state) => state.summaries);
  const appInfo = useStore((state) => state.appInfo);
  const newWorkflow = useStore((state) => state.newWorkflow);
  const openWorkflow = useStore((state) => state.openWorkflow);
  const removeWorkflow = useStore((state) => state.removeWorkflow);
  const reveal = useStore((state) => state.reveal);

  const [confirming, setConfirming] = useState<string | null>(null);

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button size="icon" variant="solid" tooltip="加载已保存的工作流">
          <FolderOpen size={14} />
        </Button>
      </PopoverTrigger>

      <PopoverContent className="loader" side="bottom" align="end" aria-label="加载工作流">
        <header className="popover__head">
          <h2 className="popover__title">加载</h2>
          <span className="popover__meta">{summaries.length} 个存档</span>
          <span className="popover__spacer" />
          <Button size="sm" variant="ghost" onClick={() => void newWorkflow()}>
            <Plus size={11} />
            新建
          </Button>
        </header>

        <div className="loader__list">
          {summaries.length === 0 && (
            <p className="hint">还没有存档。按右上角的保存按钮，把当前工作流存下来。</p>
          )}
          {summaries.map((summary) => (
            <div
              key={summary.id}
              className={`loader__item ${summary.id === meta.id ? 'is-current' : ''}`}
            >
              {confirming === summary.id ? (
                <>
                  <span className="loader__confirm">删除「{summary.name}」？</span>
                  <Button
                    size="icon-sm"
                    variant="danger"
                    tooltip="确认删除"
                    onClick={() => {
                      setConfirming(null);
                      void removeWorkflow(summary.id);
                    }}
                  >
                    <Trash2 size={11} />
                  </Button>
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    tooltip="取消"
                    onClick={() => setConfirming(null)}
                  >
                    <Undo2 size={11} />
                  </Button>
                </>
              ) : (
                <>
                  <button
                    type="button"
                    className="loader__open"
                    onClick={() => void openWorkflow(summary.id)}
                  >
                    <span className="loader__name">
                      {summary.name}
                      {summary.id === meta.id && <span className="loader__badge">当前</span>}
                    </span>
                    <span className="loader__meta">
                      {summary.nodeCount} 节点 · {summary.edgeCount} 连线 ·{' '}
                      {formatTime(summary.updatedAt)}
                    </span>
                    {summary.description && (
                      <span className="loader__desc">{summary.description}</span>
                    )}
                  </button>
                  <Button
                    size="icon-sm"
                    variant="ghost"
                    tooltip="删除"
                    onClick={() => setConfirming(summary.id)}
                  >
                    <Trash2 size={11} />
                  </Button>
                </>
              )}
            </div>
          ))}
        </div>

        {appInfo && (
          <footer className="popover__foot">
            <button type="button" className="linkish" onClick={() => void reveal(appInfo.workflowDir)}>
              打开存档目录
            </button>
          </footer>
        )}
      </PopoverContent>
    </Popover>
  );
}
