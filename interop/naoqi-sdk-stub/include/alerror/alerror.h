// Stub of the NAOqi C++ SDK, for tests only.
#pragma once
#include <stdexcept>
#include <string>

namespace AL {
class ALError : public std::runtime_error {
 public:
  ALError(const std::string& module, const std::string& method, const std::string& description)
      : std::runtime_error(module + "::" + method + ": " + description) {}
  std::string toString() const { return what(); }
};
}  // namespace AL
#define ALERROR(module, method, description) AL::ALError(module, method, description)
