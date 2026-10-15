// 生成农历对照数据：用旧版前端同一个库（lunar-javascript，pc/src/utils/lunar.ts 的依赖）
// 逐日算出 getLunarInfo 的各字段，写入同目录的 lunar_fixture.json，供 core 的 lunar 单测逐日比对。
//
// 用法（仓库根目录下，需先在 pc/ 执行过 npm ci）：
//   node pc-gpui/crates/core/tests/fixtures/gen_lunar.cjs
//
// 字段与 pc/src/utils/lunar.ts 的 getLunarInfo 一一对应。
const path = require('path')
const fs = require('fs')
const { Solar } = require(path.join(__dirname, '../../../../../pc/node_modules/lunar-javascript'))

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
