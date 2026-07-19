/** Inline error surface used inside modals/forms: a red-edged note carrying
 *  the server envelope's message verbatim (or a generic fallback). */
export function ErrorNote({ message }: { message: string }) {
  return (
    // Elevated notice: a translucent glass surface with a red left-edge accent,
    // lifted on the layered card shadow plus a soft red glow so the error reads
    // as a distinct, raised surface rather than a flat tint.
    <p className="border border-line border-l-2 border-l-red rounded-card bg-glass backdrop-blur-glass px-3 py-2 text-[12.5px] text-dim shadow-[var(--shadow-2),var(--glow-red)]">
      {message}
    </p>
  );
}
