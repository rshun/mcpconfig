export interface PairRow { key: string; value: string }
export function pairRows(map?: Record<string, string>): PairRow[] {
  return Object.entries(map ?? {}).map(([key, value]) => ({ key, value }));
}
export function pairMap(rows: PairRow[], headers = false): Record<string, string> | undefined {
  const used = new Set<string>();
  for (const row of rows) {
    const valid = headers ? /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(row.key) : /^[A-Za-z_][A-Za-z0-9_]*$/.test(row.key);
    if (!valid) throw new Error(headers ? "请求头名称无效，请检查空格或特殊字符" : "环境变量名只能使用字母、数字和下划线，且不能以数字开头");
    const id = headers ? row.key.toLowerCase() : row.key;
    if (used.has(id)) throw new Error(headers ? "请求头名称重复" : "环境变量名重复");
    used.add(id);
  }
  return rows.length ? Object.fromEntries(rows.map(({ key, value }) => [key, value])) : undefined;
}
