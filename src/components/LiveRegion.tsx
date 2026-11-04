/** Polite screen reader announcements (connection state changes). */
import { useUi } from '../stores/ui';

export function LiveRegion() {
  const announcement = useUi((s) => s.announcement);
  return (
    <div className="sr-only" aria-live="polite" aria-atomic="true">
      {/* Keyed so repeating the same text is announced again. */}
      {announcement && <span key={announcement.id}>{announcement.text}</span>}
    </div>
  );
}
