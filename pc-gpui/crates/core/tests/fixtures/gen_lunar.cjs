// 生成农历对照数据：用旧 Tauri 版前端（已删除，见 git 历史）同一个库 lunar-javascript，
// 逐日算出旧前端 `utils/lunar.ts` 里 getLunarInfo 的各字段，写入同目录的 lunar_fixture.json，供 core 的 lunar 单测逐日比对。
//
// 用法：先在任意临时目录装好与旧前端锁定版本相同的 lunar-javascript（1.7.7），再在仓库根目录用
// NODE_PATH 指向它的 node_modules 运行（不要装进仓库目录）：
//   Git Bash / Linux：
//     mkdir -p /tmp/lunar-js && npm i --prefix /tmp/lunar-js lunar-javascript@1.7.7
//     NODE_PATH=/tmp/lunar-js/node_modules node pc-gpui/crates/core/tests/fixtures/gen_lunar.cjs
//   PowerShell：
//     npm i --prefix "$env:TEMP\lunar-js" lunar-javascript@1.7.7
//     $env:NODE_PATH = "$env:TEMP\lunar-js\node_modules"; node pc-gpui/crates/core/tests/fixtures/gen_lunar.cjs
//
// 字段与旧前端 getLunarInfo 一一对应（Rust 侧见 crates/core/src/lunar.rs 的 LunarInfo）。
const path = require('path')
const fs = require('fs')
const { Solar } = require('lunar-javascript')

const START = Date.UTC(2024, 0, 1)
const END = Date.UTC(2027, 11, 31)
const DAY = 24 * 60 * 60 * 1000

const pad = (n) => String(n).padStart(2, '0')
const days = []
for (let t = START; t <= END; t += DAY) {
  const d = new Date(t)
  const [y, m, day] = [d.getUTCFullYear(), d.getUTCMonth() + 1, d.getUTCDate()]
  const lunar = Solar.fromYmd(y, m, day).getLunar()
  const festivals = lunar.getFestivals()
  days.push({
    date: `${y}-${pad(m)}-${pad(day)}`,
    dayText: lunar.getDayInChinese(),
    monthText: lunar.getMonthInChinese() + '月',
    yearText: lunar.getYearInGanZhi() + '年',
    solarTerm: lunar.getJieQi() || '',
    festival: festivals.length > 0 ? festivals[0] : '',
    isFirstDay: lunar.getDay() === 1,
    fullText: lunar.toString(),
  })
}

const out = path.join(__dirname, 'lunar_fixture.json')
// 一天一行，便于 diff
fs.writeFileSync(out, '[\n' + days.map((d) => JSON.stringify(d)).join(',\n') + '\n]\n')
console.log(`wrote ${days.length} days to ${out}`)
