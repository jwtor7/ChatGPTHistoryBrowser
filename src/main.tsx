import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { App } from './App';
import './styles.css';
import { bootstrapSessionToken } from './token';

const token = bootstrapSessionToken();
const root = document.getElementById('root');

if (!root) {
  throw new Error('Application root is unavailable.');
}

createRoot(root).render(
  <StrictMode>
    <App initialToken={token} />
  </StrictMode>,
);
