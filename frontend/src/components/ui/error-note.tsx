/** Inline error surface used inside modals/forms: a red-edged note carrying
 *  the server envelope's message verbatim (or a generic fallback). */
export function ErrorNote({ message }: { message: string }) {
  return (
    <p className="border border-line border-l-2 border-l-red rounded-card bg-[color-mix(in_oklab,var(--raise-2)_70%,transparent)] px-3 py-2 text-[12.5px] text-dim">
      {message}
    </p>
  );
}
