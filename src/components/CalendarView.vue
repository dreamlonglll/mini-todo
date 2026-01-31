<script setup lang="ts">
import { ref, computed, watch, onMounted } from 'vue'
import type { Todo } from '@/types'
import { getLunarDisplayText } from '@/utils/lunar'
import { getYearHolidays, type HolidayInfo } from '@/utils/holiday'

const props = defineProps<{
  todos: Todo[]
  isFixed?: boolean
}>()

const emit = defineEmits<{
  (e: 'select-todo', todo: Todo): void
}>()

// 当前显示的年月
const currentYear = ref(new Date().getFullYear())
const currentMonth = ref(new Date().getMonth()) // 0-11

// 当前悬停的 todo ID（用于跨行联动 hover）
const hoveredTodoId = ref<number | null>(null)

// 星期标题
const weekDays = ['一', '二', '三', '四', '五', '六', '日']

// 当前月份显示文本
const currentMonthText = computed(() => {
  return `${currentYear.value}年${currentMonth.value + 1}月`
})

// 获取某月的天数
function getDaysInMonth(year: number, month: number): number {
  return new Date(year, month + 1, 0).getDate()
}

// 获取某月第一天是星期几（0=日, 1=一, ..., 6=六）
function getFirstDayOfMonth(year: number, month: number): number {
  const day = new Date(year, month, 1).getDay()
  // 转换为周一开始 (0=一, 1=二, ..., 6=日)
  return day === 0 ? 6 : day - 1
}

// 日历格子数据
interface CalendarCell {
  date: Date
  day: number
  isCurrentMonth: boolean
  isToday: boolean
  dateStr: string // YYYY-MM-DD 格式
  row: number
  col: number
  // 农历信息
  lunarText: string
  lunarType: 'festival' | 'solarTerm' | 'lunar'
}

// 节假日数据缓存
const holidayData = ref<Map<string, HolidayInfo>>(new Map())

// 生成日历格子
const calendarCells = computed<CalendarCell[]>(() => {
  const cells: CalendarCell[] = []
  const year = currentYear.value
  const month = currentMonth.value
  
  const daysInMonth = getDaysInMonth(year, month)
  const firstDay = getFirstDayOfMonth(year, month)
  
  // 上月补齐
  const prevMonth = month === 0 ? 11 : month - 1
  const prevYear = month === 0 ? year - 1 : year
  const daysInPrevMonth = getDaysInMonth(prevYear, prevMonth)
  
  let cellIndex = 0
  
  for (let i = firstDay - 1; i >= 0; i--) {
    const day = daysInPrevMonth - i
    const date = new Date(prevYear, prevMonth, day)
    const dateStr = formatDate(date)
    const lunarDisplay = getLunarDisplayText(dateStr)
    cells.push({
      date,
      day,
      isCurrentMonth: false,
      isToday: false,
      dateStr,
      row: Math.floor(cellIndex / 7),
      col: cellIndex % 7,
      lunarText: lunarDisplay.text,
      lunarType: lunarDisplay.type
    })
    cellIndex++
  }
  
  // 当月
  const today = new Date()
  const todayStr = formatDate(today)
  
  for (let day = 1; day <= daysInMonth; day++) {
    const date = new Date(year, month, day)
    const dateStr = formatDate(date)
    const lunarDisplay = getLunarDisplayText(dateStr)
    cells.push({
      date,
      day,
      isCurrentMonth: true,
      isToday: dateStr === todayStr,
      dateStr,
      row: Math.floor(cellIndex / 7),
      col: cellIndex % 7,
      lunarText: lunarDisplay.text,
      lunarType: lunarDisplay.type
    })
    cellIndex++
  }
  
  // 下月补齐（补到6行 = 42格）
  const nextMonth = month === 11 ? 0 : month + 1
  const nextYear = month === 11 ? year + 1 : year
  const remaining = 42 - cells.length
  
  for (let day = 1; day <= remaining; day++) {
    const date = new Date(nextYear, nextMonth, day)
    const dateStr = formatDate(date)
    const lunarDisplay = getLunarDisplayText(dateStr)
    cells.push({
      date,
      day,
      isCurrentMonth: false,
      isToday: false,
      dateStr,
      row: Math.floor(cellIndex / 7),
      col: cellIndex % 7,
      lunarText: lunarDisplay.text,
      lunarType: lunarDisplay.type
    })
    cellIndex++
  }
  
  return cells
})

// 加载节假日数据
async function loadHolidayData() {
  const year = currentYear.value
  const month = currentMonth.value
  
  // 可能跨年，加载相关年份的数据
  const years = new Set<number>()
  years.add(year)
  if (month === 0) years.add(year - 1) // 一月可能显示上一年十二月
  if (month === 11) years.add(year + 1) // 十二月可能显示下一年一月
  
  const allHolidays = new Map<string, HolidayInfo>()
  
  for (const y of years) {
    const yearHolidays = await getYearHolidays(y)
    for (const [date, info] of yearHolidays) {
      allHolidays.set(date, info)
    }
  }
  
  holidayData.value = allHolidays
}

// 判断单元格是否是休息日（法定节假日）
function isCellHoliday(dateStr: string): boolean {
  const info = holidayData.value.get(dateStr)
  return info?.isHoliday ?? false
}

// 判断单元格是否是调休工作日
function isCellAdjustWorkday(dateStr: string): boolean {
  const info = holidayData.value.get(dateStr)
  return info !== null && info !== undefined && !info.isHoliday
}

// 监听年月变化，重新加载节假日数据
watch([currentYear, currentMonth], () => {
  loadHolidayData()
}, { immediate: false })

// 初始化加载节假日数据
onMounted(() => {
  loadHolidayData()
})

// 格式化日期为 YYYY-MM-DD
function formatDate(date: Date): string {
  const year = date.getFullYear()
  const month = String(date.getMonth() + 1).padStart(2, '0')
  const day = String(date.getDate()).padStart(2, '0')
  return `${year}-${month}-${day}`
}

// 获取 todo 的有效开始日期
function getTodoStartDate(todo: Todo): string {
  if (todo.startTime) {
    return todo.startTime.split('T')[0]
  }
  return todo.createdAt.split('T')[0]
}

// 获取 todo 的有效截止日期
function getTodoEndDate(todo: Todo): string | null {
  if (todo.endTime) {
    return todo.endTime.split('T')[0]
  }
  return null
}

// 跨天待办条信息
interface TodoBar {
  todo: Todo
  startCol: number
  endCol: number
  row: number
  isStart: boolean
  isEnd: boolean
  lane: number // 在同一行内的层级，用于处理重叠
}

// 检查两个待办条是否在同一行内重叠
function isBarsOverlapping(bar1: { startCol: number, endCol: number }, bar2: { startCol: number, endCol: number }): boolean {
  return !(bar1.endCol < bar2.startCol || bar2.endCol < bar1.startCol)
}

// 为一组待办条分配层级（lane），避免重叠
// 开始时间早的任务显示在上面（分配更小的 lane）
function assignLanes(bars: TodoBar[]): void {
  // 先将所有 lane 重置为 -1，表示未分配
  for (const bar of bars) {
    bar.lane = -1
  }
  
  // 按原始开始日期排序，开始时间早的排在前面
  bars.sort((a, b) => {
    const startA = getTodoStartDate(a.todo)
    const startB = getTodoStartDate(b.todo)
    if (startA !== startB) {
      return startA.localeCompare(startB)
    }
    // 如果开始日期相同，按结束日期排序
    const endA = getTodoEndDate(a.todo) || startA
    const endB = getTodoEndDate(b.todo) || startB
    return endA.localeCompare(endB)
  })
  
  for (const bar of bars) {
    // 找到不与已分配 lane 的 bar 重叠的最小 lane
    let lane = 0
    const usedLanes: Set<number> = new Set()
    
    for (const otherBar of bars) {
      if (otherBar === bar) continue
      // 只检查已经分配了 lane 的 bar（lane >= 0）
      if (otherBar.lane >= 0 && isBarsOverlapping(bar, otherBar)) {
        usedLanes.add(otherBar.lane)
      }
    }
    
    while (usedLanes.has(lane)) {
      lane++
    }
    
    bar.lane = lane
  }
}

// 计算每行的跨天待办条
const todoBarsByRow = computed(() => {
  const result: Map<number, TodoBar[]> = new Map()
  const firstDateStr = calendarCells.value[0]?.dateStr
  const lastDateStr = calendarCells.value[41]?.dateStr
  
  if (!firstDateStr || !lastDateStr) return result

  for (const todo of props.todos) {
    const startDate = getTodoStartDate(todo)
    const endDate = getTodoEndDate(todo) || startDate
    
    // 检查是否在当前日历范围内
    if (endDate < firstDateStr || startDate > lastDateStr) continue
    
    // 找到起始和结束的格子
    let startCellIndex = calendarCells.value.findIndex(c => c.dateStr === startDate)
    let endCellIndex = calendarCells.value.findIndex(c => c.dateStr === endDate)
    
    // 如果开始日期在日历范围之前
    if (startCellIndex === -1 && startDate < firstDateStr) {
      startCellIndex = 0
    }
    // 如果结束日期在日历范围之后
    if (endCellIndex === -1 && endDate > lastDateStr) {
      endCellIndex = 41
    }
    
    if (startCellIndex === -1 || endCellIndex === -1) continue
    
    const startCell = calendarCells.value[startCellIndex]
    const endCell = calendarCells.value[endCellIndex]
    
    // 按行拆分待办条
    for (let row = startCell.row; row <= endCell.row; row++) {
      const rowStartCol = (row === startCell.row) ? startCell.col : 0
      const rowEndCol = (row === endCell.row) ? endCell.col : 6
      const isStart = row === startCell.row && startDate >= firstDateStr
      const isEnd = row === endCell.row && endDate <= lastDateStr
      
      if (!result.has(row)) {
        result.set(row, [])
      }
      result.get(row)!.push({
        todo,
        startCol: rowStartCol,
        endCol: rowEndCol,
        row,
        isStart,
        isEnd,
        lane: 0 // 初始化为0，后面会重新计算
      })
    }
  }
  
  // 为每一行的待办条分配层级
  for (const [, bars] of result) {
    assignLanes(bars)
  }
  
  return result
})

// 上一月
function prevMonth() {
  if (currentMonth.value === 0) {
    currentMonth.value = 11
    currentYear.value--
  } else {
    currentMonth.value--
  }
}

// 下一月
function nextMonth() {
  if (currentMonth.value === 11) {
    currentMonth.value = 0
    currentYear.value++
  } else {
    currentMonth.value++
  }
}

// 回到今天
function goToToday() {
  const today = new Date()
  currentYear.value = today.getFullYear()
  currentMonth.value = today.getMonth()
}

// 点击待办项
function handleTodoClick(todo: Todo) {
  emit('select-todo', todo)
}

// 获取优先级颜色
function getPriorityColor(priority: string): string {
  switch (priority) {
    case 'high': return 'var(--priority-high)'
    case 'medium': return 'var(--priority-medium)'
    case 'low': return 'var(--priority-low)'
    default: return 'var(--primary)'
  }
}

// 每个待办条的高度和间距
const BAR_HEIGHT = 20
const BAR_GAP = 2
const BAR_TOP_OFFSET = 28 // 从日期数字下方开始

// 计算待办条样式
function getBarStyle(bar: TodoBar): Record<string, string> {
  const left = `calc(${bar.startCol} * (100% / 7) + 2px)`
  const width = `calc(${bar.endCol - bar.startCol + 1} * (100% / 7) - 4px)`
  
  // 根据行和层级计算 top 位置
  const laneOffset = bar.lane * (BAR_HEIGHT + BAR_GAP)
  const top = `calc(${bar.row} * (100% / 6) + ${BAR_TOP_OFFSET + laneOffset}px)`
  
  return {
    left,
    width,
    top,
    backgroundColor: getPriorityColor(bar.todo.priority)
  }
}

// 暴露方法和状态给父组件
defineExpose({
  prevMonth,
  nextMonth,
  goToToday,
  currentMonthText
})
</script>

<template>
  <div class="calendar-view" :class="{ 'fixed-mode': isFixed }">
    <!-- 星期标题 -->
    <div class="weekday-header">
      <div v-for="day in weekDays" :key="day" class="weekday-cell">
        {{ day }}
      </div>
    </div>

    <!-- 日历网格 -->
    <div class="calendar-grid">
      <!-- 日期格子 -->
      <div 
        v-for="(cell, index) in calendarCells" 
        :key="index"
        class="calendar-cell"
        :class="{ 
          'other-month': !cell.isCurrentMonth,
          'is-today': cell.isToday
        }"
      >
        <!-- 日期行：阳历 + 农历 -->
        <div class="cell-date-row">
          <!-- 左侧：阳历日期 + 班/休角标 -->
          <div class="cell-date-area">
            <span class="cell-date">{{ cell.day }}</span>
            <span v-if="isCellHoliday(cell.dateStr)" class="badge-rest">休</span>
            <span v-else-if="isCellAdjustWorkday(cell.dateStr)" class="badge-work">班</span>
          </div>
          <!-- 右侧：农历/节气/节日 -->
          <div 
            class="cell-lunar"
            :class="{
              'is-festival': cell.lunarType === 'festival',
              'is-solar-term': cell.lunarType === 'solarTerm'
            }"
          >
            {{ cell.lunarText }}
          </div>
        </div>
      </div>
      
      <!-- 跨天待办条（按行覆盖在格子上方） -->
      <template v-for="row in 6" :key="'bars-' + row">
        <div 
          v-for="(bar, barIndex) in todoBarsByRow.get(row - 1) || []"
          :key="'bar-' + row + '-' + barIndex + '-' + bar.todo.id"
          class="todo-bar"
          :class="{ 
            'is-start': bar.isStart,
            'is-end': bar.isEnd,
            'is-completed': bar.todo.completed,
            'is-hovered': hoveredTodoId === bar.todo.id
          }"
          :style="getBarStyle(bar)"
          @click.stop="handleTodoClick(bar.todo)"
          @mouseenter="hoveredTodoId = bar.todo.id"
          @mouseleave="hoveredTodoId = null"
        >
          <span v-if="bar.isStart" class="bar-title">{{ bar.todo.title }}</span>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.calendar-view {
  display: flex;
  flex-direction: column;
  height: 100%;
  background: transparent;
  border-radius: 8px;
  overflow: hidden;
}

.weekday-header {
  display: grid;
  grid-template-columns: repeat(7, 1fr);
}

.weekday-cell {
  padding: 8px 4px;
  text-align: center;
  font-size: 12px;
  font-weight: 500;
  color: var(--text-secondary);
}

.calendar-grid {
  flex: 1;
  display: grid;
  grid-template-columns: repeat(7, 1fr);
  grid-template-rows: repeat(6, 1fr);
  position: relative;
  overflow: hidden;
}

.calendar-cell {
  padding: 3px 4px;
  min-height: 60px;
  display: flex;
  flex-direction: column;
  border-right: 1px solid rgba(255, 255, 255, 0.6);
  border-bottom: 1px solid rgba(255, 255, 255, 0.6);
  position: relative;

  /* 第一行添加上边线 */
  &:nth-child(-n+7) {
    border-top: 1px solid rgba(255, 255, 255, 0.6);
  }

  &:nth-child(7n) {
    border-right: none;
  }

  &.other-month {
    .cell-date,
    .cell-lunar,
    .badge-rest,
    .badge-work {
      opacity: 0.4;
    }
  }

  &.is-today {
    .cell-date {
      background: var(--primary);
      color: white !important;
      border-radius: 50%;
      width: 20px;
      height: 20px;
      display: inline-flex;
      align-items: center;
      justify-content: center;
    }
  }
}

/* 日期行：阳历 + 农历 同行显示 */
.cell-date-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 2px;
  flex-shrink: 0;
}

/* 日期区域：阳历日期 + 班/休角标 */
.cell-date-area {
  display: flex;
  align-items: center;
  gap: 2px;
  flex-shrink: 0;
}

.cell-date {
  font-size: 13px;
  font-weight: 500;
  color: var(--text-primary);
  flex-shrink: 0;
  line-height: 1.4;
}

/* 班/休角标 */
.badge-rest,
.badge-work {
  font-size: 9px;
  font-weight: 600;
  line-height: 1;
  padding: 1px 2px;
  border-radius: 2px;
}

.badge-rest {
  color: white;
  background: #EF4444;
}

.badge-work {
  color: white;
  background: #F59E0B;
}

/* 农历日期 - 靠右显示 */
.cell-lunar {
  font-size: 10px;
  color: var(--text-tertiary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  line-height: 1.4;
  flex-shrink: 1;
  text-align: right;

  /* 传统节日 */
  &.is-festival {
    color: #EF4444;
    font-weight: 500;
  }

  /* 节气 */
  &.is-solar-term {
    color: #10B981;
    font-weight: 500;
  }
}

/* 跨天待办条 */
.todo-bar {
  position: absolute;
  height: 20px;
  border-radius: 4px;
  cursor: pointer;
  display: flex;
  align-items: center;
  padding: 0 6px;
  transition: opacity 0.2s, transform 0.1s;
  z-index: 10;
  overflow: hidden;

  /* 联动 hover 效果 */
  &.is-hovered {
    opacity: 0.85;
    transform: scale(1.02);
    z-index: 15;
  }

  &.is-completed {
    opacity: 0.5;

    .bar-title {
      text-decoration: line-through;
    }

    &.is-hovered {
      opacity: 0.65;
    }
  }
}

.bar-title {
  font-size: 11px;
  color: white;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  text-shadow: 0 1px 2px rgba(0, 0, 0, 0.2);
}

/* 固定模式样式 */
.calendar-view.fixed-mode {
  .calendar-cell {
    border-bottom-color: rgba(255, 255, 255, 0.6);
  }

  .cell-date {
    color: var(--text-primary);
  }

  .cell-lunar {
    color: var(--text-tertiary);
    
    &.is-festival {
      color: #EF4444;
    }
    
    &.is-solar-term {
      color: #10B981;
    }
  }

  .weekday-cell {
    color: var(--text-secondary);
  }

  /* 固定模式下角标样式保持 */
  .badge-rest {
    color: white;
    background: #EF4444;
  }

  .badge-work {
    color: white;
    background: #F59E0B;
  }
}
</style>
