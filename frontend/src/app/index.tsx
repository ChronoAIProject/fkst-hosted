import { createBrowserRouter, Navigate, RouterProvider } from 'react-router-dom';
import { Shell } from './shell';
import { Introduction } from '../pages/introduction';
import { GetStarted } from '../pages/get-started';

// The whole site is static — two content routes under one shell. No data
// providers, no auth gate, no API. The router is created once at module load
// so navigation state survives re-renders.
const router = createBrowserRouter(
  [
    {
      path: '/',
      element: <Shell />,
      children: [
        { path: '', element: <Introduction /> },
        { path: 'get-started', element: <GetStarted /> },
        // Unknown paths fall back to the landing page.
        { path: '*', element: <Navigate to="/" replace /> },
      ],
    },
  ],
  {
    future: {
      v7_relativeSplatPath: true,
    },
  }
);

export function App() {
  return <RouterProvider router={router} future={{ v7_startTransition: true }} />;
}
