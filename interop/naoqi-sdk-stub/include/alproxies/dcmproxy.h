// Stub of the NAOqi C++ SDK, for tests only: records the aliases and commands it is given.
#pragma once
#include <alvalue/alvalue.h>
#include <vector>

namespace AL {
class DCMProxy {
 public:
  DCMProxy() : fTime(1000) {}
  ALValue createAlias(const ALValue& alias);
  void setAlias(const ALValue& command);
  int getTime(const int& offset) { return fTime + offset; }
  // Test state.
  int fTime;
  std::vector<ALValue> fAliases;
  std::vector<ALValue> fCommands;
};
}  // namespace AL
