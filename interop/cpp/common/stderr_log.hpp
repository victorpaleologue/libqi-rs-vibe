#pragma once
// libqi's default console log handler prints to *stdout*, which every interop
// program reserves for machine-readable output. This replaces it with a
// handler writing to stderr.

#include <iostream>
#include <string>

#include <qi/clock.hpp>
#include <qi/log.hpp>

namespace interop
{
  inline void routeQiLogsToStderr(qi::LogLevel level = qi::LogLevel_Warning)
  {
    qi::log::removeHandler("consoleloghandler");
    qi::log::addHandler(
      "interop-stderr",
      [](const qi::LogLevel lv,
         const qi::Clock::time_point,
         const qi::SystemClock::time_point,
         const char* category,
         const char* msg,
         const char* /*file*/,
         const char* /*fct*/,
         int /*line*/) {
        std::cerr << "[qi " << qi::log::logLevelToString(lv) << "] " << (category ? category : "") << ": "
                  << (msg ? msg : "") << "\n";
      },
      level);
  }
}
