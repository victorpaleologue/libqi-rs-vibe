// Stub of the NAOqi C++ SDK, for tests only: the subset of AL::ALValue the DCM bridge uses.
#pragma once
#include <string>
#include <vector>

namespace AL {
class ALValue {
 public:
  enum Type { TypeInvalid, TypeInt, TypeFloat, TypeString, TypeArray };
  ALValue() : fType(TypeInvalid), fInt(0), fFloat(0) {}
  ALValue(int value) : fType(TypeInt), fInt(value), fFloat(0) {}
  ALValue(float value) : fType(TypeFloat), fInt(0), fFloat(value) {}
  ALValue(const std::string& value) : fType(TypeString), fInt(0), fFloat(0), fString(value) {}
  void arraySetSize(int size) { fType = TypeArray; fArray.resize(size); }
  int getSize() const { return fType == TypeArray ? static_cast<int>(fArray.size()) : 0; }
  ALValue& operator[](int i) { return fArray.at(i); }
  const ALValue& operator[](int i) const { return fArray.at(i); }
  Type getType() const { return fType; }
  operator int() const { return fType == TypeFloat ? static_cast<int>(fFloat) : fInt; }
  operator float() const { return fType == TypeInt ? static_cast<float>(fInt) : fFloat; }
  operator std::string() const { return fString; }
  std::string toString() const;

 private:
  Type fType;
  int fInt;
  float fFloat;
  std::string fString;
  std::vector<ALValue> fArray;
};
}  // namespace AL
