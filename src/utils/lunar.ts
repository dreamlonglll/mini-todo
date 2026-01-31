/**
 * 农历工具模块
 * 使用 lunar-javascript 库进行公历转农历、获取节气和传统节日
 */
import { Solar, Lunar } from 'lunar-javascript'

export interface LunarInfo {
  /** 农历日期显示文本（如：初一、初二、十五） */
  dayText: string
  /** 农历月份（如：正月、二月） */
  monthText: string
  /** 农历年份（如：甲辰年） */
  yearText: string
  /** 节气（如：立春、清明），无则为空 */
  solarTerm: string
  /** 传统节日（如：春节、中秋节），无则为空 */
  festival: string
  /** 是否是农历初一（用于显示月份） */
  isFirstDay: boolean
  /** 完整农历日期（如：二零二四年正月初一） */
  fullText: string
}

/**
 * 获取指定日期的农历信息
 * @param date 公历日期对象或 YYYY-MM-DD 格式字符串
 */
export function getLunarInfo(date: Date | string): LunarInfo {
  let solar: InstanceType<typeof Solar>
  
  if (typeof date === 'string') {
    const [year, month, day] = date.split('-').map(Number)
    solar = Solar.fromYmd(year, month, day)
  } else {
    solar = Solar.fromDate(date)
  }
  
  const lunar = solar.getLunar() as InstanceType<typeof Lunar>
  
  // 获取农历日期文本
  const dayText = lunar.getDayInChinese()
  const monthText = lunar.getMonthInChinese() + '月'
  const yearText = lunar.getYearInGanZhi() + '年'
  
  // 判断是否是初一
  const isFirstDay = lunar.getDay() === 1
  
  // 获取节气
  const solarTerm = lunar.getJieQi() || ''
  
  // 获取传统节日
  const festivals = lunar.getFestivals() as string[]
  const festival = festivals.length > 0 ? festivals[0] : ''
  
  // 完整农历日期
  const fullText = lunar.toString()
  
  return {
    dayText,
    monthText,
    yearText,
    solarTerm,
    festival,
    isFirstDay,
    fullText
  }
}

/**
 * 获取日历单元格应显示的文本
 * 优先级：传统节日 > 节气 > 农历日期（初一显示月份）
 * @param date 公历日期
 */
export function getLunarDisplayText(date: Date | string): { text: string; type: 'festival' | 'solarTerm' | 'lunar' } {
  const info = getLunarInfo(date)
  
  // 优先显示传统节日
  if (info.festival) {
    return { text: info.festival, type: 'festival' }
  }
  
  // 其次显示节气
  if (info.solarTerm) {
    return { text: info.solarTerm, type: 'solarTerm' }
  }
  
  // 最后显示农历日期（初一显示月份）
  const lunarText = info.isFirstDay ? info.monthText : info.dayText
  return { text: lunarText, type: 'lunar' }
}

/**
 * 批量获取一个月的农历信息（用于日历视图优化）
 * @param year 年份
 * @param month 月份（0-11）
 */
export function getMonthLunarInfo(year: number, month: number): Map<string, LunarInfo> {
  const result = new Map<string, LunarInfo>()
  
  // 获取当月天数
  const daysInMonth = new Date(year, month + 1, 0).getDate()
  
  // 为每一天计算农历信息
  for (let day = 1; day <= daysInMonth; day++) {
    const dateStr = `${year}-${String(month + 1).padStart(2, '0')}-${String(day).padStart(2, '0')}`
    result.set(dateStr, getLunarInfo(dateStr))
  }
  
  return result
}
