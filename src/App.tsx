import { ReactFlowProvider } from '@xyflow/react';
import { useEffect } from 'react';

import { Canvas } from './graph/Canvas';
import { SlashOverlay } from './graph/SlashOverlay';
import { TopLeftChrome, TopRightChrome } from './panels/Chrome';
import { RunStatus } from './panels/RunStatus';
import { useStore } from './state/store';
import { TooltipProvider } from './ui/Tooltip';

export function App() {
  const ready = useStore((state) => state.ready);
  const init = useStore((state) => state.init);

  useEffect(() => {
    void init();
  }, [init]);

  return (
    <TooltipProvider>
      <ReactFlowProvider>
        <div className="app">
          <Canvas />
          {ready && (
            <>
              <TopLeftChrome />
              <TopRightChrome />
              <RunStatus />
              <SlashOverlay />
            </>
          )}
        </div>
      </ReactFlowProvider>
    </TooltipProvider>
  );
}
