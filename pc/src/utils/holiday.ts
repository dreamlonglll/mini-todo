/**
 * 节假日服务模块
 * 通过 Tauri 后端调用 NateScarlet/holiday-cn 开源数据获取中国法定节假日
 * 后端带有本地文件缓存，前端带有内存缓存和重试机制
 */
import { invoke } from '@tauri-apps/api/core'

export interface HolidayInfo {
  /** 日期 YYYY-MM-DD */
  date: string
  /** 是否是休息日 */
  isHoliday: boolean
  /** 是否是工作日（包括调休） */
  isWorkday: boolean
  /** 是否是调休日（周末需要上班） */
  isAdjust: boolean
  /** 节日名称（如：春节、国庆节），非节日则为空 */
  name: string
  /** 节日类型：1-法定节假日 2-调休上班 */
  type: number
}

// 后端返回的节假日数据格式
interface BackendHolidayInfo {
  date: string
  name: string
  isOffDay: boolean
}

// 年度节假日内存缓存
const holidayCache = new Map<number, Map<string, HolidayInfo>>()

// 最大重试次数
const MAX_RETRIES = 3
// 重试基础延迟（毫秒）
const RETRY_BASE_DELAY = 2000

/**
 * 延迟指定时间
 */
function delay(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms))
}

/**
 * 从 Tauri 后端获取指定年份的节假日数据（带重试机制）
 * @param year 年份
 */
async function fetchYearHolidays(year: number): Promise<Map<string, HolidayInfo>> {
  for (let attempt = 1; attempt <= MAX_RETRIES; attempt++) {
    try {
      const holidays = await invoke<BackendHolidayInfo[]>('fetch_holidays', { year })
      const result = new Map<string, HolidayInfo>()
      
      for (const info of holidays) {
        result.set(info.date, {
          date: info.date,
          isHoliday: info.isOffDay,
          isWorkday: !info.isOffDay,
          isAdjust: !info.isOffDay, // 如果不是休息日但在节假日数据中，说明是调休上班
          name: info.name,
          type: info.isOffDay ? 1 : 2
        })
      }
      
      return result
    } catch (error) {
      console.warn(`[Holiday] Attempt ${attempt}/${MAX_RETRIES} failed for year ${year}:`, error)
      if (attempt < MAX_RETRIES) {
        // 递增延迟重试
        await delay(RETRY_BASE_DELAY * attempt)
      }
    }
  }
  
  console.error(`[Holiday] All ${MAX_RETRIES} attempts failed for year ${year}`)
  return new Map()
}

/**
 * 获取指定年份的节假日数据（带内存缓存）
 * @param year 年份
 */
export async function getYearHolidays(year: number): Promise<Map<string, HolidayInfo>> {
  // 检查内存缓存
  if (holidayCache.has(year)) {
    return holidayCache.get(year)!
  }
  
  // 从后端获取（后端自带本地文件缓存 + 网络请求）
  const holidays = await fetchYearHolidays(year)
  
  // 存入内存缓存（仅缓存有效数据）
  if (holidays.size > 0) {
    holidayCache.set(year, holidays)
  }
  
  return holidays
}

