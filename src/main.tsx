import { createRoot } from 'react-dom/client';

import '@xyflow/react/dist/style.css';
import './styles/app.css';

import { App } from './App';
import { installDevConsole } from './lib/dev-console';

if (import.meta.env.DEV) installDevConsole();

const container = document.getElementById('root');
if (!container) throw new Error('找不到 #root 容器');

createRoot(container).render(<App />);
