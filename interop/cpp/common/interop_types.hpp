#pragma once
// Struct types shared by the interop programs. They are registered with the
// libqi type system through QI_TYPE_STRUCT, which yields annotated tuple
// signatures such as "(ii)<Point2D,x,y>" (see libqi
// tests/messaging/test_binarycoder.cpp for the reference expectations).

#include <qi/type/typeinterface.hpp>

struct Point2D
{
  int x = 0;
  int y = 0;
};
QI_TYPE_STRUCT(Point2D, x, y);

struct TimeStamp
{
  int i = 0;
  int j = 0;
};
QI_TYPE_STRUCT(TimeStamp, i, j);

struct TimeStampedPoint2D
{
  Point2D p;
  TimeStamp t;
};
QI_TYPE_STRUCT(TimeStampedPoint2D, p, t);
