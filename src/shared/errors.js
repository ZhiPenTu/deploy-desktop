/**
 * 把异常转换成可以直接展示的中文文案。
 * 直接拼接 Error 对象会带上 "Error:" 前缀，对使用者没有意义。
 */
export function errorText(error, fallback = '操作失败，请稍后重试。') {
  const text = typeof error === 'string' ? error : error?.message;
  const trimmed = typeof text === 'string' ? text.trim() : '';
  return trimmed || fallback;
}
