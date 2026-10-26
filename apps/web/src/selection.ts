import type { McpSelection } from "./types";

/** 保留每个单元格的配对关系，不能把行列各自合并后再做笛卡尔积。 */
export function selectionFromCells(cells: Iterable<string>): McpSelection {
  const selection: McpSelection = {};
  for (const cell of cells) {
    const separator = cell.lastIndexOf("::");
    if (separator < 1 || separator + 2 === cell.length) {
      throw new Error("无效的 MCP 选择");
    }
    const name = cell.slice(0, separator);
    const clientId = cell.slice(separator + 2);
    const names = (selection[clientId] ??= []);
    if (!names.includes(name)) names.push(name);
  }
  return selection;
}
