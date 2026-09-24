export const MAX_PROJECT_NAME_LENGTH = 100;

export function validateProjectName(input: string): string | undefined {
  const name = input.trim();
  if (!name) return 'Vui lòng nhập tên dự án.';
  if (Array.from(name).length > MAX_PROJECT_NAME_LENGTH) {
    return `Tên dự án không được dài quá ${MAX_PROJECT_NAME_LENGTH} ký tự.`;
  }
  if ([...name].some((character) => /[\u0000-\u001f\u007f]/u.test(character))) {
    return 'Tên dự án không được chứa ký tự điều khiển.';
  }
  return undefined;
}
