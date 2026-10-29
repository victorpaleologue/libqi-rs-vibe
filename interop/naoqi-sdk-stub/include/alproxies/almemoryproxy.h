// Stub of the NAOqi C++ SDK, for tests only: the storage of the keys of ALMemory.
#pragma once
#include <map>
#include <string>

namespace AL {
class ALMemoryProxy {
 public:
  void* getDataPtr(const std::string& key);
  // Test state: the floats of the keys.
  std::map<std::string, float> fData;
};
}  // namespace AL
