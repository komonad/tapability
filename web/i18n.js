// Translations for the web front end.
//
// The engine only ever reports language-neutral status codes plus their numbers
// (`statusKey` / `statusArgs`), so everything the player reads comes from here.
// Simplified Chinese is the default language; the choice is remembered in
// localStorage under `tapa.lang`.
//
// Placeholders are `{0}`, `{1}`, ... and are filled with the status arguments.

window.TAPA_I18N = {
  "zh-CN": {
    name: "简体中文",
    intro:
      "给每一格涂黑或留白。带数字的格子必须为空，数字是它周围 8 格中黑色连续段的长度" +
      "（顺时针读，段与段之间至少隔一个白格）；所有黑格必须连成一片，且不能出现 2×2 全黑。" +
      "每道题都保证恰好一个解。",
    "hint.mouse": "左键：墙 · 右键：空 · 再点同一个标记：清除 · 拖拽：一笔画",
    "hint.keys":
      "Shift + 悬停：高亮整块墙 · 中键或 Alt+左键点线索：只推这一个线索 · Z：撤销（长按连撤）",
    "hint.touch": "触屏：轻点格子循环 墙 → 空 → 清除 · 拖动一笔画 · 轻点线索推该线索",
    "panel.controls": "操作",
    "panel.generation": "生成参数",
    "panel.batch": "批量工具",
    "btn.new": "新题目",
    "btn.clear": "清空",
    "btn.check": "检查",
    "btn.solution": "显示解答",
    "btn.onestep": "单步推理",
    "btn.cluestep": "推这一个线索",
    "btn.undo": "撤销",
    "btn.apply": "应用并重新生成",
    "btn.defaults": "恢复默认",
    "btn.print": "打印 1 题",
    "btn.bench": "测速 3 题",
    "batch.help": "两者都在 WebAssembly 里、worker 线程上运行，页面不会卡住。",
    "help.default": "把鼠标移到参数上可以看到说明。",
    "zoom.label": "格子大小 (px)",
    "zoom.fit": "自适应",
    "zoom.help":
      "一个格子画多少像素。它改变的是画面大小，不是格子数量。“自适应”按窗口铺满。",
    "output.close": "关闭",
    "output.print": "打印：1 道题",
    "output.bench": "测速：{0} 道题",
    "info.seed": "种子 {0}    {1} 条线索",
    "info.broken": "   红 = 违反规则",
    "stats": "本次生成：{0} 条线索 · {1}×{2} · {3} ms · {4} 次尝试 · {5} 节点",
    "message.saved": "已保存在浏览器本地",

    "status.idle": "点“新题目”开始。",
    "busy.working": "正在 WebAssembly 里计算…",
    "status.generating": "正在生成一道唯一解的题目…",
    "status.generation_failed": "生成失败（种子 {0}）：换小一点的棋盘，或把预算调大。",
    "status.cells_left": "还剩 {0} 格",
    "status.solved": "解开了！按 N 换一道。",
    "status.filled_with_errors": "已填满，但有 {0} 处违反规则——按 C 看详情",
    "status.nothing_to_undo": "没有可撤销的步骤。",
    "status.solution_shown": "正在显示解答（按 S 隐藏）",
    "status.solution_hidden": "解答已隐藏",
    "status.one_step_none": "单步：现在线索推不出新东西。",
    "status.one_step_filled": "单步：根据线索填了 {0} 格。",
    "status.clue_only": "中键点线索格（或 Alt+左键 / 用“推这一个线索”按钮）才能单推该线索。",
    "status.clue_unsatisfiable": "线索 ({0},{1}) 已经无法满足了。",
    "status.clue_forces_nothing": "线索 ({0},{1}) 推不出新东西。",
    "status.clue_filled": "线索 ({0},{1})：填了 {2} 格。",
    "status.check_ok": "目前没有错误。",
    "status.check_wrong": "{0} 格标错了（红色叉）",
    "status.bench_done": "测速：{0} 道题，用时 {1} 毫秒（见下方输出）",
    "status.print_done": "已打印 {0} 道题（见下方输出）",
    "status.hover_clue": "先把鼠标移到线索格上，再推它。",

    "field.size.label": "棋盘大小 (3-60)",
    "field.size.help": "棋盘是 size × size 格。改动它会重建布局并开始一道新题。",
    "field.density.label": "密度 LO-HI",
    "field.density.help": "成为墙的格子比例，在这个区间里随机取值。调高会更满。",
    "field.max_clueless_fraction.label": "最大无线索占比",
    "field.max_clueless_fraction.help":
      "当某个完全没有线索的连片区域超过棋盘的这个比例时，就丢弃这个形状。",
    "field.node_budget.label": "节点预算",
    "field.node_budget.help": "一次生成允许的搜索节点数，所有唯一性证明共享。",
    "field.time_budget_ms.label": "时间预算 (ms)",
    "field.time_budget_ms.help": "一次生成允许的毫秒数。0 表示不限制。",
    "field.check_min_nodes.label": "单个证明最少节点",
    "field.check_min_nodes.help": "单个唯一性证明至少拿到的节点数，避免便宜的被饿死。",
    "field.check_slack.label": "预算余量",
    "field.check_slack.help": "单个证明能超出自己那份多少倍。1 = 严格平均分配。",
    "field.max_attempts.label": "最大尝试次数",
    "field.max_attempts.help": "放弃前最多尝试多少个随机形状。",
    "field.seed.label": "种子（留空 = 随机）",
    "field.seed.help": "固定种子让题目可复现。留空则按时间取一个新种子。",
  },

  en: {
    name: "English",
    intro:
      "Fill every cell black or white. A numbered cell stays empty, and its numbers are the " +
      "lengths of the black runs around it, read clockwise (runs are separated by at least one " +
      "white cell); all black cells must be connected and no 2x2 block may be all black. Every " +
      "puzzle has exactly one solution.",
    "hint.mouse":
      "Left click: wall · right click: empty · click the same mark again: clear · drag: paint a stroke",
    "hint.keys":
      "Shift + hover: spotlight a wall group · middle click or Alt + click a clue: step just it · Z: undo (hold to repeat)",
    "hint.touch":
      "Touch: tap a cell to cycle wall → empty → clear · drag to paint a stroke · tap a clue to step it",
    "panel.controls": "Controls",
    "panel.generation": "Generation",
    "panel.batch": "Batch tools",
    "btn.new": "New puzzle",
    "btn.clear": "Clear",
    "btn.check": "Check",
    "btn.solution": "Solution",
    "btn.onestep": "One step",
    "btn.cluestep": "Step one clue",
    "btn.undo": "Undo",
    "btn.apply": "Apply & new puzzle",
    "btn.defaults": "Defaults",
    "btn.print": "Print 1",
    "btn.bench": "Bench 3",
    "batch.help": "Both run inside WebAssembly on the worker thread, so the board stays responsive.",
    "help.default": "Hover a setting to read what it does.",
    "zoom.label": "Cell size (px)",
    "zoom.fit": "Fit",
    "zoom.help":
      "How many pixels one board cell is drawn at. It changes the size of the picture, not the " +
      "number of cells. Fit draws the board as large as the window allows.",
    "output.close": "Close",
    "output.print": "Print: 1 puzzle",
    "output.bench": "Bench: {0} generations",
    "info.seed": "seed {0}    {1} clues",
    "info.broken": "   red = rule broken",
    "stats": "last generation: {0} clues · {1}×{2} · {3} ms · {4} attempt(s) · {5} node(s)",
    "message.saved": "saved in this browser",

    "status.idle": "Press New puzzle to generate one.",
    "busy.working": "Working in WebAssembly...",
    "status.generating": "Generating a puzzle with a unique solution...",
    "status.generation_failed":
      "generation failed (seed {0}); try a smaller board or a bigger budget",
    "status.cells_left": "{0} cell(s) left",
    "status.solved": "Solved! Press N for a new puzzle.",
    "status.filled_with_errors": "Filled, but {0} rule violation(s) - press C to see them",
    "status.nothing_to_undo": "Nothing to undo.",
    "status.solution_shown": "Showing the solution (S to hide)",
    "status.solution_hidden": "Solution hidden",
    "status.one_step_none": "One step: the clues force nothing new right now.",
    "status.one_step_filled": "One step: filled {0} cell(s) the clues force.",
    "status.clue_only": "Middle-click a clue cell to step just that clue.",
    "status.clue_unsatisfiable": "Clue ({0},{1}) can no longer be satisfied.",
    "status.clue_forces_nothing": "Clue ({0},{1}) forces nothing new.",
    "status.clue_filled": "Clue ({0},{1}): filled {2} cell(s).",
    "status.check_ok": "No mistakes so far.",
    "status.check_wrong": "{0} wrong cell(s) marked in red",
    "status.bench_done": "Bench: {0} puzzle(s) in {1} ms (see the output below)",
    "status.print_done": "Printed {0} puzzle(s) below",
    "status.hover_clue": "Hover a clue cell first, then step it.",

    "field.size.label": "Board size (3-60)",
    "field.size.help":
      "Board is size x size cells. Changing it rebuilds the layout and starts a new puzzle.",
    "field.density.label": "Density LO-HI",
    "field.density.help":
      "Share of cells that become walls, picked randomly in this range. Higher fills the board more.",
    "field.max_clueless_fraction.label": "Max clueless fraction",
    "field.max_clueless_fraction.help":
      "Reject a shape when its biggest patch with no clue at all is larger than this share of the board.",
    "field.node_budget.label": "Node budget",
    "field.node_budget.help":
      "Search nodes allowed for one whole generation, shared between all uniqueness proofs.",
    "field.time_budget_ms.label": "Time budget (ms)",
    "field.time_budget_ms.help":
      "Milliseconds allowed for one whole generation. 0 means no time limit.",
    "field.check_min_nodes.label": "Min nodes per proof",
    "field.check_min_nodes.help":
      "Smallest node budget a single uniqueness proof always gets, so cheap proofs are never starved.",
    "field.check_slack.label": "Budget slack",
    "field.check_slack.help":
      "How much one proof may borrow over its fair share of the pool. 1 = strictly equal shares.",
    "field.max_attempts.label": "Max attempts",
    "field.max_attempts.help": "How many random shapes to try before giving up on this configuration.",
    "field.seed.label": "Seed (blank = random)",
    "field.seed.help":
      "A fixed seed makes puzzles reproducible. Blank picks a new seed from the clock.",
  },
};
