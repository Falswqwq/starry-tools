/** 画布上的两组浮动控件：左上角是「这个工作流」，右上角是「拿它做什么」。 */

import { Loader, Play, Save } from 'lucide-react';

import { useStore } from '../state/store';
import { Button } from '../ui/Button';
import { DescriptionMenu } from './DescriptionMenu';
import { NodeLibrary } from './NodeLibrary';
import { WorkflowMenu } from './WorkflowMenu';

export function TopLeftChrome() {
  const meta = useStore((state) => state.meta);
  const setMeta = useStore((state) => state.setMeta);

  // 未保存的提示点不在这儿 —— 它挂在右上角的保存按钮下面。
  // 夹在工作流名和说明按钮中间会把这一串读断。
  return (
    <div className="toolbar toolbar--left">
      <NodeLibrary />
      <input
        className="wfname"
        value={meta.name}
        aria-label="工作流名称"
        spellCheck={false}
        placeholder="未命名工作流"
        onChange={(event) => setMeta({ name: event.target.value })}
      />
      <DescriptionMenu />
    </div>
  );
}

export function TopRightChrome() {
  const dirty = useStore((state) => state.dirty);
  const save = useStore((state) => state.save);
  const run = useStore((state) => state.run);
  const running = useStore((state) => state.running);
  const errors = useStore(
    (state) => state.resolved?.issues.filter((issue) => issue.severity === 'error').length ?? 0,
  );
  const firstBadNode = useStore(
    (state) =>
      state.resolved?.issues.find((issue) => issue.severity === 'error' && issue.nodeId)?.nodeId ??
      null,
  );
  const select = useStore((state) => state.select);

  return (
    <div className="toolbar toolbar--right">
      {errors > 0 && (
        <button
          type="button"
          className="chip chip--bad nodrag"
          onClick={() => select(firstBadNode)}
          title="跳到第一个有问题的节点"
        >
          {errors} 处问题
        </button>
      )}
      <WorkflowMenu />
      {/* 保存按钮底下挂一个点，表示有改动还没落盘 */}
      <span className="save-slot">
        <Button size="icon" variant="solid" tooltip="保存" onClick={() => void save()}>
          <Save size={14} />
        </Button>
        <span
          className={`dirty ${dirty ? 'is-on' : ''}`}
          title={dirty ? '有未保存的改动' : '已保存'}
        />
      </span>
      <Button size="sm" variant="primary" disabled={running} onClick={() => void run()}>
        {running ? <Loader size={12} className="spin" /> : <Play size={12} />}
        {running ? '运行中' : '运行'}
      </Button>
    </div>
  );
}
