#pragma once
// Minimal JSON emission helpers (no external dependency). Only what the
// interop programs need: escaped strings, numbers, booleans, arrays and
// objects assembled from already-serialized fragments.

#include <cstdint>
#include <cstdio>
#include <string>
#include <vector>

namespace ijson
{
  inline std::string str(const std::string& s)
  {
    std::string out;
    out.reserve(s.size() + 2);
    out += '"';
    for (unsigned char c : s)
    {
      switch (c)
      {
      case '"': out += "\\\""; break;
      case '\\': out += "\\\\"; break;
      case '\n': out += "\\n"; break;
      case '\r': out += "\\r"; break;
      case '\t': out += "\\t"; break;
      default:
        if (c < 0x20)
        {
          char buf[8];
          std::snprintf(buf, sizeof(buf), "\\u%04x", c);
          out += buf;
        }
        else
        {
          out += static_cast<char>(c);
        }
      }
    }
    out += '"';
    return out;
  }

  inline std::string boolean(bool b) { return b ? "true" : "false"; }
  inline std::string num(long long v) { return std::to_string(v); }
  inline std::string num(unsigned long long v) { return std::to_string(v); }
  inline std::string num(int v) { return std::to_string(v); }
  inline std::string num(unsigned v) { return std::to_string(v); }
  inline std::string num(double v)
  {
    char buf[64];
    std::snprintf(buf, sizeof(buf), "%.17g", v);
    return buf;
  }

  inline std::string array(const std::vector<std::string>& fragments)
  {
    std::string out = "[";
    for (size_t i = 0; i < fragments.size(); ++i)
    {
      if (i) out += ",";
      out += fragments[i];
    }
    out += "]";
    return out;
  }

  /// Build an object from (key, already-serialized value) pairs, in order.
  inline std::string object(const std::vector<std::pair<std::string, std::string>>& fields)
  {
    std::string out = "{";
    for (size_t i = 0; i < fields.size(); ++i)
    {
      if (i) out += ",";
      out += str(fields[i].first);
      out += ":";
      out += fields[i].second;
    }
    out += "}";
    return out;
  }

  inline std::string hex(const std::vector<std::uint8_t>& bytes)
  {
    static const char* digits = "0123456789abcdef";
    std::string out;
    out.reserve(bytes.size() * 2);
    for (std::uint8_t b : bytes)
    {
      out += digits[b >> 4];
      out += digits[b & 0xF];
    }
    return out;
  }
}
