import { describe, expect, it } from 'vitest';
import { validateProjectName } from './project-name';

describe('validateProjectName', () => {
  it('accepts a trimmed project name', () => {
    expect(validateProjectName('  Tập phim  ')).toBeUndefined();
  });

  it('rejects an empty name and whitespace-only values', () => {
    expect(validateProjectName('')).toContain('nhập');
    expect(validateProjectName('   ')).toContain('nhập');
  });

  it('limits names to 100 Unicode characters', () => {
    expect(validateProjectName('😀'.repeat(100))).toBeUndefined();
    expect(validateProjectName('😀'.repeat(101))).toContain('100');
  });

  it('rejects control characters', () => {
    expect(validateProjectName('Project\nname')).toContain('ký tự điều khiển');
  });
});
