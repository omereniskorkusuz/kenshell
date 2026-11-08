import { beforeEach, describe, expect, it } from 'vitest';
import { useUpdate } from './update';

const info = { version: '0.2.0', current: '0.1.0', notes: 'Notes', date: null };

describe('update prompt state', () => {
  beforeEach(() => useUpdate.setState({ info: null, dismissed: false, phase: 'available' }));

  it('shows a found update until "Later"', () => {
    useUpdate.getState().found(info);
    expect(useUpdate.getState()).toMatchObject({ info, dismissed: false, phase: 'available' });
    useUpdate.getState().later();
    expect(useUpdate.getState().dismissed).toBe(true);
    // A later check (Settings → Check now) shows it again.
    useUpdate.getState().found(info);
    expect(useUpdate.getState().dismissed).toBe(false);
  });

  it('tracks download progress and failure', () => {
    useUpdate.getState().found(info);
    useUpdate.getState().progress(50, 200);
    expect(useUpdate.getState()).toMatchObject({ phase: 'downloading', percent: 25 });
    useUpdate.getState().progress(10, null);
    expect(useUpdate.getState().percent).toBeNull();
    useUpdate.getState().failed('network');
    expect(useUpdate.getState()).toMatchObject({ phase: 'error', error: 'network' });
  });
});
